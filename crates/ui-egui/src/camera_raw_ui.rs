//! Filter › Camera Raw Filter…: a large dialog laid out as Photoshop 25.4's Camera Raw 16.1.1
//! (measured at 100 %): a top bar with the file name, the preview on the left with its zoom and
//! Before/After bar, on the right the histogram, the active tool's panels (Edit: Light, Color,
//! Effects, Curve, Color Mixer, Color Grading, Detail, Optics, Geometry,
//! Calibration; Remove, Masking, Red Eye, Presets) and Camera Raw's tool strip, OK / Cancel below.
//! The panels are in [`crate::camera_raw_panels`], the other tools in
//! [`crate::camera_raw_tools_ui`], the look in [`crate::camera_raw_style`].
//!
//! The preview runs [`photocraft_algo::camera_raw::develop`] on a CPU proxy of the layer
//! (≤ 900 px, `pixel_scale` keeps pixel radii true to the full image), recomputed when a
//! control changes. OK runs `filter.cameraRaw` with the non-default settings, so the result is
//! one history step (or a smart filter on a smart object) and exactly replayable.
//!
//! Control channel: `ui.menu.invoke {"id":"filter.cameraRaw","params":{"ui":{"set":{…},
//! "commit":true | "cancel":true}}}`; the reply describes the dialog. `{"ui":{"saveSettings":true}}`
//! and `{"ui":{"loadSettings":true}}` run Save Settings… / Load Settings… (Camera Raw `.xmp`
//! settings files, see [`photocraft_io::camera_raw_xmp`]) through the file dialog.

use egui::{Align2, Color32, FontId, Rect as ERect, Sense, Stroke, TextureHandle, pos2, vec2};
use photocraft_algo::camera_raw::CameraRaw;
use photocraft_algo::histogram::RgbHistogram;

use crate::camera_raw_scope_ui::MAX_SAMPLERS;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

const PROXY_SIDE: usize = 900;
/// Measured column widths (pt): the panels with their scroll bar, the tool strip, the bars.
const PANEL_W: f32 = 270.0;
const STRIP_W: f32 = 40.0;
const TOP_H: f32 = 39.0;
const FOOTER_H: f32 = 58.0;
const PREVIEW_BAR_H: f32 = 40.0;
const HISTOGRAM_H: f32 = 122.0;
const INFO_H: f32 = 26.0;

/// The preview's Before / After views, cycled with Q as in Camera Raw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BeforeAfter {
    #[default]
    Off,
    LeftRight,
    LeftRightSplit,
    TopBottom,
    TopBottomSplit,
}

impl BeforeAfter {
    fn next(self) -> Self {
        match self {
            Self::Off => Self::LeftRight,
            Self::LeftRight => Self::LeftRightSplit,
            Self::LeftRightSplit => Self::TopBottom,
            Self::TopBottom => Self::TopBottomSplit,
            Self::TopBottomSplit => Self::Off,
        }
    }
}

pub struct CameraRawDialog {
    pub layer: LayerId,
    layer_name: String,
    /// The document's name, shown in Camera Raw's top bar.
    doc_name: String,
    pub params: CameraRaw,
    /// Settings when the dialog opened (stored settings of an edited smart filter).
    pub(crate) original: CameraRaw,
    target: Target,
    pub(crate) curve_state: crate::state::PointCurveState,
    pub(crate) curve_rect: Option<ERect>,
    /// Panels whose eye is off: the preview leaves their settings out (OK still applies them).
    pub(crate) hidden: Vec<String>,
    pub(crate) profile_browser: bool,
    /// The White Balance tool: the next click on the preview neutralises that colour.
    pub(crate) wb_picker: bool,
    pub(crate) mono_tex: Option<TextureHandle>,
    pub(crate) before_after: BeforeAfter,
    /// The Before view's settings (none at first); the Before/After bar copies and swaps them.
    pub(crate) before_params: CameraRaw,
    before_dirty: bool,
    pub(crate) tools: crate::camera_raw_tools_ui::ToolState,
    pub(crate) prefs_open: bool,
    pub(crate) prefs_tab: usize,
    /// A text field had focus last frame: the Enter / Esc that ended its edit is not the
    /// dialog's (OK / Cancel).
    text_focus_prev: bool,
    /// Proxy pixels (straight RGBA) and size.
    pub(crate) proxy: Vec<[f32; 4]>,
    pub(crate) selection: Option<Vec<f32>>,
    pub(crate) pw: usize,
    pub(crate) ph: usize,
    full_w: usize,
    full_h: usize,
    preview_ppp: f32,
    pub(crate) detail: super::camera_raw_detail_ui::DetailPreview,
    display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(crate) viewport: Option<ERect>,
    pub(crate) float: bool,
    pub(crate) tex: Option<TextureHandle>,
    before_tex: Option<TextureHandle>,
    before_display_key: Option<u64>,
    pub(crate) before_histogram: RgbHistogram,
    pub(crate) processed: Vec<[f32; 4]>,
    pub(crate) lab_transform: std::sync::Arc<photocraft_cms::Transform>,
    pub(crate) scope_transform: std::sync::Arc<photocraft_cms::Transform>,
    pub(crate) scope: super::camera_raw_scope_ui::ScopeView,
    pub(crate) after_histogram: RgbHistogram,
    pub(crate) preview_revision: u64,
    histogram_ms: f64,
    pub(crate) dirty: bool,
    pub show_before: bool,
    pub render_ms: f64,
    /// Set for the open-time dialog ([`Target::OpenRaw`]).
    pub(crate) raw_open: Option<RawOpen>,
    /// The open-time dialog's camera tone curve: Shadows / Highlights act on the raw data there,
    /// in linear light before this curve, and the preview undoes it to show the same.
    raw_curve: Option<Vec<[f32; 2]>>,
    /// Settings before each finished edit, so Ctrl+Z works inside the dialog (#4100).
    undo: Vec<CameraRaw>,
    redo: Vec<CameraRaw>,
    /// Params at pointer-down. One drag is one undo step.
    edit_origin: Option<CameraRaw>,
}

impl CameraRawDialog {
    pub fn describe(&self, view: &crate::state::CameraRawScopeState, navigation: &crate::state::CameraRawPreviewState) -> Value {
        let h = self.histogram();
        let mut result = json!({"layer": self.layer.0, "params": serde_json::to_value(&self.params).unwrap_or(Value::Null), "proxy": [self.pw, self.ph],
            "renderMs": self.render_ms, "histogramMs": self.histogram_ms, "previewRevision": self.preview_revision, "before": self.show_before,
            "histogram": {"source": if self.show_before { "before" } else { "after" }, "approximate": true, "size": [self.pw, self.ph],
                "sampleDomain": "preview-rgb", "range": [0, 1], "red": h.channels[0].as_slice(), "green": h.channels[1].as_slice(), "blue": h.channels[2].as_slice(),
                "samples": h.samples, "transparent": h.transparent, "invalid": h.invalid, "underflow": h.underflow, "overflow": h.overflow, "shadows": h.shadows, "highlights": h.highlights}});
        result["smartFilter"] = json!(match self.target {
            Target::SmartFilter(index) => Some(index),
            Target::NewFilter | Target::OpenRaw => None,
        });
        result["openingRaw"] = json!(self.raw_open.as_ref().map(|r| json!({"name": r.name, "path": r.path, "demosaic": r.demosaic.id()})));
        result["view"] = json!(navigation);
        result["sourceSize"] = json!([self.full_w, self.full_h]);
        result["previewApproximate"] = json!(!self.detail.ready && (self.pw != self.full_w || self.ph != self.full_h));
        result["detailPending"] = json!(self.detail.pending());
        result["detailError"] = json!(self.detail.error);
        result["viewportRect"] = json!(self.viewport.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["zoom"] = json!(self.viewport.map(|r| navigation.scale(r, vec2(self.full_w as f32, self.full_h as f32) / self.preview_ppp)));
        result["scope"] = serde_json::to_value(view).unwrap_or(Value::Null);
        result["hasScopeSelection"] = json!(self.selection.is_some());
        result["vectorscope"] = json!(
            self.scope
                .cache
                .as_ref()
                .map(|(_, before, selected, data)| json!({"samples":data.samples,"before":before,"selectedRegion":selected,"bins":data.counts}))
        );
        result["scopeMs"] = json!(self.scope.ms);
        result["scopeRevision"] = json!(self.scope.revision);
        result["overlayRevision"] = json!(self.scope.overlay_key.map(|k| k.0));
        result["pointerReadout"] = self.scope.pointer_sample.and_then(|p| self.readout(p, view.lab)).unwrap_or(Value::Null);
        result["samplerReadouts"] = json!(view.samplers.iter().take(MAX_SAMPLERS).map(|p| self.readout(*p, view.lab)).collect::<Vec<_>>());
        result["hoveredZone"] = json!(self.scope.hovered_zone);
        result["previewRect"] = json!(self.scope.preview_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["hidden"] = json!(self.hidden);
        result["beforeAfter"] = json!(self.before_after);
        result["profileBrowser"] = json!(self.profile_browser);
        result["whiteBalancePicker"] = json!(self.wb_picker);
        result["curveState"] = json!(self.curve_state);
        result["curveRect"] = json!(self.curve_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["hoverSample"] = json!(self.hover_sample(view.vectorscope));
        result["vectorscopeRect"] = json!(self.scope.vectorscope_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["scopeRect"] = json!(self.scope.rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result
    }

    pub(crate) fn histogram(&self) -> &RgbHistogram {
        if self.show_before { &self.before_histogram } else { &self.after_histogram }
    }

    /// Params for the engine: only the settings that differ from the defaults.
    pub fn command_params(&self) -> Value {
        non_defaults(&self.params)
    }

    fn image(px: &[[f32; 4]], w: usize, h: usize, display: &photocraft_engine::display_color::CanvasDisplay) -> egui::ColorImage {
        // Bands copied straight from `px`, converted in parallel (#4682), not a whole copy of the
        // proxy converted on one thread.
        super::camera_raw_detail_ui::display_image(Rect::new(0, 0, w as i32, h as i32), display, |r, out| {
            let start = (r.y0.max(0) as usize).saturating_mul(w);
            if let Some(src) = px.get(start..start.saturating_add(out.len())) {
                out.copy_from_slice(src);
            }
        })
    }

    pub(crate) fn render(&mut self, ctx: &egui::Context) {
        let t0 = crate::gpu_canvas::now_ms();
        let mut px = self.proxy.clone();
        let mut p = crate::camera_raw_panels::visible(&self.params, &self.hidden);
        p.pixel_scale = (self.pw as f32 / self.full_w.max(1) as f32).min(1.0);
        if let Some(curve) = &self.raw_curve {
            let tone = raw_stage(&p);
            photocraft_io::raw::retone(&mut px, self.pw, self.ph, curve, tone);
            (p.shadows, p.highlights, p.clarity, p.texture, p.dehaze) = (0.0, 0.0, 0.0, 0.0, 0.0);
        }
        photocraft_algo::camera_raw::develop(&mut px, self.pw, self.ph, &p, self.float);
        if let Some(mask) = &self.selection {
            for ((out, original), coverage) in px.iter_mut().zip(&self.proxy).zip(mask) {
                for (out, original) in out.iter_mut().zip(original) {
                    *out = *original + (*out - *original) * coverage;
                }
            }
        }
        let histogram_start = crate::gpu_canvas::now_ms();
        self.after_histogram = RgbHistogram::from_rgba(&px);
        self.histogram_ms = crate::gpu_canvas::now_ms() - histogram_start;
        let img = Self::image(&px, self.pw, self.ph, &self.display);
        match &mut self.tex {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => self.tex = Some(ctx.load_texture("camera-raw-after", img, egui::TextureOptions::LINEAR)),
        }
        if self.before_tex.is_none() || self.before_dirty || self.before_display_key != Some(self.display.key) {
            let mut before = self.proxy.clone();
            let mut bp = self.before_params.clone();
            bp.pixel_scale = p.pixel_scale;
            bp.encoding_gamma = self.params.encoding_gamma;
            photocraft_algo::camera_raw::develop(&mut before, self.pw, self.ph, &bp, self.float);
            self.before_histogram = RgbHistogram::from_rgba(&before);
            let img = Self::image(&before, self.pw, self.ph, &self.display);
            match &mut self.before_tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.before_tex = Some(ctx.load_texture("camera-raw-before", img, egui::TextureOptions::LINEAR)),
            }
            self.before_dirty = false;
            self.before_display_key = Some(self.display.key);
        }
        if self.profile_browser {
            let gray = CameraRaw { monochrome: true, ..p.clone() };
            let mut mono = self.proxy.clone();
            photocraft_algo::camera_raw::develop(&mut mono, self.pw, self.ph, &gray, self.float);
            let img = Self::image(&mono, self.pw, self.ph, &self.display);
            match &mut self.mono_tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.mono_tex = Some(ctx.load_texture("camera-raw-mono", img, egui::TextureOptions::LINEAR)),
            }
        }
        self.processed = px;
        self.scope.cache = None;
        self.render_ms = crate::gpu_canvas::now_ms() - t0;
        self.preview_revision = self.preview_revision.saturating_add(1);
        self.dirty = false;
    }
}

/// Opens the dialog on the active layer.
/// Where OK writes: a new filter on the layer, or an existing smart filter (by index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    NewFilter,
    SmartFilter(usize),
    /// The open-time dialog of a raw file just opened (File › Open): OK is **Open**, Cancel
    /// closes the document again. See [`RawOpen`].
    OpenRaw,
}

/// A raw file opened interactively, shown in Camera Raw before it is kept (as Photoshop opens
/// raws in Adobe Camera Raw first). The document is already developed with the defaults; Open
/// re-develops it when Temperature, Tint, Exposure, Shadows, Highlights, Clarity, Texture,
/// Dehaze or the demosaic method changed (those act on the sensor data, see
/// [`photocraft_io::raw::import_raw_tuned`]) and applies the other settings as one Camera Raw
/// step, then marks the document unmodified. Cancel closes it.
#[derive(Clone, Debug)]
pub struct RawOpen {
    pub document: photocraft_doc::DocId,
    pub name: String,
    pub path: Option<String>,
    /// The file, when it was opened from bytes (no path to read it again from).
    pub bytes: Option<std::sync::Arc<Vec<u8>>>,
    /// The demosaic method the open developed it with.
    pub developed_with: photocraft_io::raw::Demosaic,
    /// The method chosen in the dialog: Open re-develops when it differs from `developed_with`.
    pub demosaic: photocraft_io::raw::Demosaic,
}

/// Should an interactive open of `name` show the open-time Camera Raw dialog? `warnings` are
/// the import's: only a raw developed from its sensor data qualifies (not a preview fallback).
pub fn wants_open_dialog(app: &PhotocraftApp, warnings: &[String]) -> bool {
    app.session.prefs().raw_defaults.open_in_camera_raw && warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE))
}

/// `warnings` without the "developed with default settings" note: the open-time dialog shows
/// how the raw is developed.
pub fn without_develop_note(warnings: &[String]) -> Vec<String> {
    warnings.iter().filter(|w| !w.contains(photocraft_io::raw::DEVELOPED_NOTE)).cloned().collect()
}

/// Queues the open-time dialog for the active document (shown on the next frame).
pub fn queue_open_dialog(app: &mut PhotocraftApp, name: &str, path: Option<&str>, bytes: Option<&[u8]>) {
    if let Some(st) = app.session.active() {
        // The open developed it with the preference's method (`Preferences::import_options`).
        let method = app.session.prefs().raw_defaults.demosaic.method();
        app.pending_raw_open = Some(RawOpen {
            document: st.doc.id,
            name: name.to_string(),
            path: path.map(str::to_string),
            bytes: bytes.map(|b| std::sync::Arc::new(b.to_vec())),
            developed_with: method,
            demosaic: method,
        });
    }
}

/// Opens the queued open-time dialog on its document's (only) layer.
fn open_raw(app: &mut PhotocraftApp, ctx: &egui::Context, raw: RawOpen) -> Result<(), String> {
    let index = document_index(app, raw.document).ok_or("the raw document was closed")?;
    app.session.set_active(index);
    let (layer, surf, _) = crate::distort_ui::active_pixels(app)?;
    open_pixels(app, ctx, layer, surf, CameraRaw::default(), Coverage::None, Target::OpenRaw)?;
    let curve = match (&raw.bytes, &raw.path) {
        (Some(b), _) => Some(photocraft_io::raw::tone_curve(b)),
        #[cfg(not(target_arch = "wasm32"))]
        (None, Some(p)) => std::fs::read(p).ok().map(|b| photocraft_io::raw::tone_curve(&b)),
        _ => None,
    };
    if let Some(d) = app.camera_raw.as_mut() {
        d.layer_name = raw.name.clone();
        d.detail.raw_curve = curve.clone().map(std::sync::Arc::new);
        d.raw_curve = curve;
        d.raw_open = Some(raw);
    }
    Ok(())
}

fn document_index(app: &PhotocraftApp, id: photocraft_doc::DocId) -> Option<usize> {
    app.session.documents().iter().position(|st| st.doc.id == id)
}

/// The non-default settings of `p` as engine params.
fn non_defaults(p: &CameraRaw) -> Value {
    let full = serde_json::to_value(p).unwrap_or(json!({}));
    let def = serde_json::to_value(CameraRaw::default()).unwrap_or(json!({}));
    let mut out = serde_json::Map::new();
    if let (Value::Object(f), Value::Object(d)) = (full, def) {
        for (k, v) in f {
            if k != "pixelScale" && d.get(&k) != Some(&v) {
                out.insert(k, v);
            }
        }
    }
    Value::Object(out)
}

/// The job id of an open-time re-develop (not an engine command: the shell starts it).
pub const REDEVELOP_JOB: &str = "cameraRaw.redevelop";

/// An open-time re-develop running in the background: what to finish with when it lands.
#[derive(Clone, Debug)]
pub struct Redevelop {
    pub job: photocraft_engine::jobs::JobId,
    params: CameraRaw,
    raw: RawOpen,
    /// The job is the final Camera Raw step (`filter.cameraRaw`), not the re-develop: when it
    /// lands the document is marked unmodified.
    filter_step: bool,
}

/// The sliders a raw open applies while developing, before the tone curve (as Camera Raw does).
fn raw_stage(p: &CameraRaw) -> photocraft_io::raw::Tone {
    photocraft_io::raw::Tone { shadows: p.shadows, highlights: p.highlights, clarity: p.clarity, texture: p.texture, dehaze: p.dehaze }
}

/// Open: re-develop for raw-stage settings, apply the rest, keep the document unmodified.
///
/// Re-developing reads and decodes the whole raw again, so in the desktop app (background jobs
/// on) it runs as a job like the open itself: the reply is `{"job", "pending": true}`, the
/// document is locked meanwhile, and [`on_redevelop_event`] finishes when it lands. Elsewhere
/// (tests, the web) it runs inline.
fn commit_open_raw(app: &mut PhotocraftApp, params: &CameraRaw, raw: &RawOpen) -> Result<Value, String> {
    let index = document_index(app, raw.document).ok_or("the raw document was closed")?;
    let tuning = photocraft_io::raw::RawTuning {
        temperature: params.temperature,
        tint: params.tint,
        exposure: params.exposure,
        demosaic: raw.demosaic,
        shadows: params.shadows,
        highlights: params.highlights,
        clarity: params.clarity,
        texture: params.texture,
        dehaze: params.dehaze,
    };
    let as_opened = photocraft_io::raw::RawTuning { demosaic: raw.developed_with, ..Default::default() };
    // Without the file the raw-stage settings stay RGB adjustments, like the filter's.
    let source = match (&raw.bytes, &raw.path) {
        (Some(b), _) => Some(photocraft_engine::jobs::OpenSource::Bytes(b.clone())),
        #[cfg(not(target_arch = "wasm32"))]
        (None, Some(p)) => Some(photocraft_engine::jobs::OpenSource::Path(p.clone())),
        _ => None,
    };
    let Some(source) = source.filter(|_| tuning != as_opened) else {
        return finish_open_raw(app, index, params.clone(), None, Some(raw));
    };
    app.session.set_active(index);
    let (name, doc_id) = (raw.name.clone(), raw.document);
    let work = move |ctx: &photocraft_engine::jobs::JobCtx| -> photocraft_engine::Result<(photocraft_io::ImportResult, bool)> {
        ctx.progress(0.0, "Reading");
        let bytes = match source {
            photocraft_engine::jobs::OpenSource::Bytes(b) => b,
            #[cfg(not(target_arch = "wasm32"))]
            photocraft_engine::jobs::OpenSource::Path(p) => {
                std::sync::Arc::new(std::fs::read(&p).map_err(|e| photocraft_engine::EngineError::Other(format!("{p}: {e}")))?)
            }
            #[cfg(target_arch = "wasm32")]
            photocraft_engine::jobs::OpenSource::Path(p) => return Err(photocraft_engine::EngineError::Other(format!("{p}: cannot read files here"))),
        };
        ctx.check()?;
        ctx.progress(0.05, "Developing");
        photocraft_io::raw::import_raw_tuned(&name, &bytes, &tuning).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
    };
    let apply = move |s: &mut photocraft_engine::Session, (r, wb): (photocraft_io::ImportResult, bool)| replace_raw_document(s, doc_id, r, wb);
    let label = crate::i18n::fmt(tl!("Developing {name}"), &[("name", &raw.name)]);
    let started = if app.background_jobs {
        app.session.start_job(REDEVELOP_JOB, json!({"name": raw.name}), &label, true, work, apply).map_err(|e| e.to_string())?
    } else {
        let v = work(&photocraft_engine::jobs::JobCtx::new()).and_then(|t| apply(&mut app.session, t)).map_err(|e| e.to_string())?;
        photocraft_engine::jobs::Started::Done(v)
    };
    match started {
        photocraft_engine::jobs::Started::Done(v) => finish_redevelop(app, params, raw, &v),
        photocraft_engine::jobs::Started::Job(job) => {
            app.raw_redevelop = Some(Redevelop { job, params: params.clone(), raw: raw.clone(), filter_step: false });
            app.ui.status = label;
            app.ui.status_error = false;
            Ok(json!({"job": job.0, "pending": true}))
        }
    }
}

/// Swap the default-developed document `doc_id` for the re-developed one, at the same tab
/// position and with the same path. Returns `{document, previous, warnings, wb}`.
fn replace_raw_document(
    s: &mut photocraft_engine::Session,
    doc_id: photocraft_doc::DocId,
    r: photocraft_io::ImportResult,
    wb: bool,
) -> photocraft_engine::Result<Value> {
    let index =
        s.documents().iter().position(|st| st.doc.id == doc_id).ok_or_else(|| photocraft_engine::EngineError::Other("the raw document was closed".into()))?;
    let path = s.documents().get(index).and_then(|st| st.path.clone());
    s.close(index);
    let (opened, _) = s.open_document(r.document, path);
    let index = match s.execute("document.move", json!({"document": opened, "to": index})) {
        Ok(moved) => moved.get("document").and_then(Value::as_u64).map_or(opened, |i| i as usize),
        Err(_) => opened,
    };
    s.set_active(index);
    Ok(json!({"document": index, "previous": doc_id.0, "warnings": r.warnings, "wb": wb}))
}

/// After the re-develop: keep the old document's view (zoom, scroll) on the new one, report
/// the import's notes, then apply the remaining settings.
fn finish_redevelop(app: &mut PhotocraftApp, params: &CameraRaw, raw: &RawOpen, v: &Value) -> Result<Value, String> {
    let index = v.get("document").and_then(Value::as_u64).ok_or("the re-develop returned no document")? as usize;
    if let Some(new_id) = app.session.documents().get(index).map(|st| st.doc.id) {
        for id in app.view_docs.iter_mut().filter(|id| **id == raw.document) {
            *id = new_id;
        }
    }
    let warnings: Vec<String> =
        v.get("warnings").and_then(Value::as_array).map(|a| a.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default();
    crate::notices::io_warnings(app, &format!("Opened {}", raw.name), &without_develop_note(&warnings));
    let mut rest = params.clone();
    rest.exposure = 0.0;
    rest.shadows = 0.0;
    rest.highlights = 0.0;
    rest.clarity = 0.0;
    rest.texture = 0.0;
    rest.dehaze = 0.0;
    if v.get("wb").and_then(Value::as_bool) == Some(true) {
        rest.temperature = 0.0;
        rest.tint = 0.0;
    }
    finish_open_raw(app, index, rest, Some(true), Some(raw))
}

/// The other settings as one Camera Raw step on document `index`, then the document is marked
/// unmodified (developing is part of opening). The step runs like any filter (in the
/// background in the desktop app); the document is marked unmodified when it lands.
fn finish_open_raw(app: &mut PhotocraftApp, index: usize, rest: CameraRaw, redeveloped: Option<bool>, raw: Option<&RawOpen>) -> Result<Value, String> {
    app.session.set_active(index);
    app.sync_views();
    let mut result = json!({"document": index, "redeveloped": redeveloped.unwrap_or(false)});
    if !rest.is_identity() {
        let layer = app.session.active().and_then(|st| st.active_layer).ok_or("no layer to develop")?;
        let mut p = non_defaults(&rest);
        p["layer"] = json!(layer.0);
        let r = app.run("filter.cameraRaw", p)?;
        if r.get("pending").and_then(Value::as_bool) == Some(true)
            && let (Some(job), Some(raw)) = (r.get("job").and_then(Value::as_u64), raw)
        {
            app.raw_redevelop = Some(Redevelop { job: photocraft_engine::jobs::JobId(job), params: rest.clone(), raw: raw.clone(), filter_step: true });
        }
        result["filter"] = r;
    }
    mark_unmodified(app, index);
    Ok(result)
}

fn mark_unmodified(app: &mut PhotocraftApp, index: usize) {
    let active = app.session.active_index();
    app.session.set_active(index);
    if let Some(st) = app.session.active_mut() {
        st.saved_revision = st.revision;
    }
    if let Some(i) = active {
        app.session.set_active(i);
    }
    app.sync_views();
}

/// A finished background re-develop ([`REDEVELOP_JOB`]) or open-time Camera Raw step. `true`
/// when the event was one of those (handled here).
pub fn on_redevelop_event(app: &mut PhotocraftApp, e: &photocraft_engine::jobs::JobEvent) -> bool {
    let Some(pending) = app.raw_redevelop.take_if(|r| r.job == e.id) else { return false };
    if pending.filter_step {
        if matches!(e.outcome, photocraft_engine::jobs::JobOutcome::Done(_))
            && let Some(index) = e.document.and_then(|id| document_index(app, id))
        {
            mark_unmodified(app, index);
        }
        // The usual filter reporting still applies.
        return false;
    }
    match &e.outcome {
        photocraft_engine::jobs::JobOutcome::Done(v) => match finish_redevelop(app, &pending.params, &pending.raw, v) {
            Ok(_) => {
                app.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", &pending.raw.name)]);
                app.ui.status_error = false;
            }
            Err(err) => crate::notices::error(app, format!("{}: {err}", e.label)),
        },
        photocraft_engine::jobs::JobOutcome::Failed(err) => {
            // The document stays open, developed with the defaults.
            app.sync_views();
            crate::notices::error(app, format!("{}: {err}", e.label));
        }
        photocraft_engine::jobs::JobOutcome::Cancelled => {
            app.ui.status = crate::i18n::fmt(tl!("Cancelled {label}"), &[("label", &e.label)]);
            app.ui.status_error = false;
        }
    }
    true
}

/// Cancel: closes the dialog; for the open-time dialog also the document it was opening.
fn cancel(app: &mut PhotocraftApp) {
    let Some(d) = app.camera_raw.take() else { return };
    if let (Target::OpenRaw, Some(raw)) = (d.target, &d.raw_open)
        && let Some(index) = document_index(app, raw.document)
    {
        app.session.close(index);
        app.sync_views();
        app.ui.status = crate::i18n::fmt(tl!("Cancelled opening {name}"), &[("name", &raw.name)]);
        app.ui.status_error = false;
    }
}

/// What limits the preview: the document selection (a new filter applies through it, as the
/// engine does), a smart object's filter mask, or nothing.
enum Coverage {
    Selection,
    Mask(photocraft_doc::LayerMask),
    None,
}

pub fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    photocraft_engine::commands::find("filter.cameraRaw").map(|c| (c.enabled)(&app.session)).unwrap_or(Err("unknown command".into()))?;
    let (layer, surf, _) = crate::distort_ui::active_pixels(app)?;
    open_pixels(app, ctx, layer, surf, CameraRaw::default(), Coverage::Selection, Target::NewFilter)
}

/// Opens Camera Raw on smart filter `index` of smart layer `layer`, with its stored settings and
/// the pixels below it; OK updates that filter (`layer.smartFilter.setParams`), as double-clicking
/// a Camera Raw smart filter does in Photoshop.
pub fn open_smart_filter(app: &mut PhotocraftApp, ctx: &egui::Context, layer: LayerId, index: usize) -> Result<(), String> {
    use photocraft_engine::lens_cmds::{RAW, raw_params};
    let st = app.session.active().ok_or("no document")?;
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else {
        return Err("the layer is not a smart object".into());
    };
    let filter = sm.smart_filters.get(index).ok_or("no such smart filter")?;
    if filter.command != RAW {
        return Err("the smart filter is not a Camera Raw filter".into());
    }
    let params = raw_params(RAW, &filter.params).map_err(|e| e.to_string())?;
    let surf = photocraft_engine::smart_cmds::render_below_filter(&st.doc, sm, index)
        .map_err(|e| e.to_string())?
        .ok_or("the smart object's contents are unavailable (missing linked file?)")?;
    let coverage = sm.filter_mask.clone().filter(|m| m.enabled).map_or(Coverage::None, Coverage::Mask);
    open_pixels(app, ctx, layer, surf, params, coverage, Target::SmartFilter(index))
}

fn open_pixels(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    layer: LayerId,
    surf: photocraft_raster::Surface,
    params: CameraRaw,
    coverage: Coverage,
    target: Target,
) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let canvas = st.doc.bounds();
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let mut params = params;
    if params.encoding_gamma == 0.0 {
        params.encoding_gamma = photocraft_engine::lens_cmds::encoding_gamma(&st.doc);
    }
    // Use the engine filter domain, including pixels outside the canvas: spatial Camera Raw
    // stages (vignette, grain, local contrast) must agree with the committed result.
    let area = canvas.union(&surf.content_bounds());
    let width = area.x1.checked_sub(area.x0).filter(|n| (1..=1_048_576).contains(n));
    let height = area.y1.checked_sub(area.y0).filter(|n| (1..=1_048_576).contains(n));
    let (Some(w), Some(h)) = (width, height) else {
        return Err("Camera Raw preview bounds are too large".into());
    };
    let (w, h) = (w as usize, h as usize);
    if w.checked_mul(h).is_none_or(|pixels| pixels > 512_000_000) {
        return Err("Camera Raw preview bounds are too large".into());
    }
    let k = w.max(h).div_ceil(PROXY_SIDE).max(1);
    let (pw, ph) = (w.div_ceil(k), h.div_ceil(k));
    let mut proxy = vec![[0.0f32; 4]; pw * ph];
    let mut row = vec![[0.0f32; 4]; w];
    for py in 0..ph {
        // Box-average k×k blocks.
        let mut acc = vec![[0.0f32; 5]; pw];
        for dy in 0..k {
            let y = area.y0 + (py * k + dy).min(h - 1) as i32;
            surf.read_rgba_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
            for (x, q) in row.iter().enumerate() {
                let a = &mut acc[x / k];
                for c in 0..4 {
                    a[c] += q[c];
                }
                a[4] += 1.0;
            }
        }
        for (x, a) in acc.iter().enumerate() {
            proxy[py * pw + x] = [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]];
        }
    }
    // Coverage at the centre of each proxy block.
    let sample = |value: &dyn Fn(i32, i32) -> f32| {
        (0..ph)
            .flat_map(|y| (0..pw).map(move |x| (area.x0 + (x * k + k / 2).min(w - 1) as i32, area.y0 + (y * k + k / 2).min(h - 1) as i32)))
            .map(|(x, y)| value(x, y).clamp(0.0, 1.0))
            .collect::<Vec<_>>()
    };
    let selection = match &coverage {
        Coverage::Selection => st.doc.selection.as_ref().map(|mask| sample(&|x, y| mask.sample_channel(x, y, 0))),
        Coverage::Mask(mask) => Some(sample(&|x, y| mask.value(x, y))),
        Coverage::None => None,
    };
    let source = photocraft_engine::color_cmds::composite_profile(&st.doc);
    let display = app.session.color.canvas_display(&st.doc).map_err(|e| e.to_string())?;
    let lab_transform = photocraft_cms::cached(&source, photocraft_cms::Builtin::LabD50.profile(), Default::default()).map_err(|e| e.to_string())?;
    let scope_transform = photocraft_cms::cached(&source, photocraft_cms::Builtin::Srgb.profile(), Default::default()).map_err(|e| e.to_string())?;
    if let Some(saved) = app.session.prefs().dialogs.get("filter.cameraRaw.scope")
        && saved.get("samplers").and_then(Value::as_array).is_none_or(|a| a.len() <= MAX_SAMPLERS)
        && let Ok(view) = serde_json::from_value(saved.clone())
    {
        app.ui.camera_raw_scope = view;
    }
    if let Some(saved) = app.session.prefs().dialogs.get(LAYOUT_KEY)
        && let Ok(layout) = serde_json::from_value::<crate::state::CameraRawLayoutState>(saved.clone())
    {
        app.ui.camera_raw_layout = layout;
    }
    app.ui.camera_raw_preview = Default::default();
    app.ui.camera_raw_scope.samplers.clear();
    // Keep Camera Raw compact on open; vectorscope is an opt-in context-menu view.
    app.ui.camera_raw_scope.vectorscope = false;
    if selection.is_none() {
        app.ui.camera_raw_scope.selected_region = false;
    }
    let float = surf.format().sample == photocraft_color::SampleType::F32;
    let mut d = CameraRawDialog {
        layer,
        layer_name: name,
        doc_name: st.doc.name.clone(),
        original: params.clone(),
        params,
        target,
        before_histogram: RgbHistogram::from_rgba(&proxy),
        processed: Vec::new(),
        lab_transform,
        scope_transform,
        scope: Default::default(),
        after_histogram: RgbHistogram::default(),
        preview_revision: 0,
        histogram_ms: 0.0,
        proxy,
        selection,
        pw,
        ph,
        full_w: w,
        full_h: h,
        preview_ppp: ctx.pixels_per_point(),
        detail: super::camera_raw_detail_ui::DetailPreview::new(
            surf,
            area,
            match coverage {
                Coverage::Selection => {
                    st.doc.selection.clone().map(super::camera_raw_detail_ui::Coverage::Selection).unwrap_or(super::camera_raw_detail_ui::Coverage::None)
                }
                Coverage::Mask(mask) => super::camera_raw_detail_ui::Coverage::Mask(mask),
                Coverage::None => super::camera_raw_detail_ui::Coverage::None,
            },
        ),
        display,
        viewport: None,
        float,
        tex: None,
        before_tex: None,
        before_display_key: None,
        dirty: true,
        show_before: false,
        curve_state: Default::default(),
        curve_rect: None,
        hidden: Vec::new(),
        profile_browser: false,
        wb_picker: false,
        mono_tex: None,
        before_after: BeforeAfter::Off,
        before_params: CameraRaw::default(),
        before_dirty: false,
        tools: Default::default(),
        prefs_open: false,
        prefs_tab: 0,
        text_focus_prev: false,
        render_ms: 0.0,
        raw_open: None,
        raw_curve: None,
        undo: Vec::new(),
        redo: Vec::new(),
        edit_origin: None,
    };
    d.render(ctx);
    app.camera_raw = Some(d);
    Ok(())
}

/// Menu / control-channel entry point. `None` when the call isn't for this dialog.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "filter.cameraRaw" {
        return None;
    }
    let Some(fields) = params.as_object() else {
        return Some(Err("Camera Raw params must be an object".into()));
    };
    if fields.is_empty() {
        return Some(
            open(app, ctx).map(|_| app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)).unwrap_or(Value::Null)),
        );
    }
    if let Some(sf) = params.get("smartFilter") {
        let opened = (|| {
            if fields.len() > 1 {
                return Err("smartFilter opens the dialog; send ui requests separately".to_string());
            }
            let layer = sf.get("layer").and_then(Value::as_u64).ok_or("smartFilter.layer must be a layer id")?;
            let index = sf.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or("smartFilter.index must be a filter index")?;
            open_smart_filter(app, ctx, LayerId(layer), index)?;
            Ok(app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)).unwrap_or(Value::Null))
        })();
        return Some(opened);
    }
    let ui = params.get("ui")?;
    let Some(fields) = ui.as_object() else {
        return Some(Err("Camera Raw ui must be an object".into()));
    };
    for (key, value) in fields {
        match key.as_str() {
            "set" | "scope" | "view" | "layout" | "hidden" | "beforeAfter" | "demosaic" => {}
            "before" | "commit" | "cancel" | "profileBrowser" | "saveSettings" | "loadSettings" | "undo" | "redo" if value.is_boolean() => {}
            "before" | "commit" | "cancel" | "profileBrowser" | "saveSettings" | "loadSettings" | "undo" | "redo" => {
                return Some(Err(format!("Camera Raw {key} must be a boolean")));
            }
            _ => return Some(Err(format!("unknown Camera Raw ui property {key}"))),
        }
    }
    if ui.get("commit").and_then(Value::as_bool) == Some(true) && ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        return Some(Err("Camera Raw cannot commit and cancel together".into()));
    }
    for (key, run) in [("saveSettings", save_settings as fn(&mut PhotocraftApp) -> Result<Value, String>), ("loadSettings", load_settings)] {
        if ui.get(key).and_then(Value::as_bool) == Some(true) {
            if fields.len() > 1 {
                return Some(Err(format!("Camera Raw {key} must be sent on its own")));
            }
            return Some(run(app));
        }
    }
    let was_closed = app.camera_raw.is_none();
    let previous_view = app.ui.camera_raw_scope.clone();
    let previous_navigation = app.ui.camera_raw_preview;
    if was_closed && let Err(e) = open(app, ctx) {
        return Some(Err(e));
    }
    let result = menu_update(app, ctx, ui);
    if was_closed && result.as_ref().is_some_and(Result::is_err) {
        app.camera_raw = None;
        app.ui.camera_raw_scope = previous_view;
        app.ui.camera_raw_preview = previous_navigation;
    }
    result
}

fn menu_update(app: &mut PhotocraftApp, ctx: &egui::Context, ui: &Value) -> Option<Result<Value, String>> {
    // Stage both filter and presentation updates. No texture, preference or dialog mutation
    // occurs until every part of the request has passed validation.
    let d = app.camera_raw.as_ref()?;
    let mut next_params = d.params.clone();
    if let Some(set) = ui.get("set") {
        if !set.is_object() {
            return Some(Err("bad Camera Raw settings: expected an object".into()));
        }
        let mut cur = serde_json::to_value(&d.params).unwrap_or(json!({}));
        if let (Value::Object(c), Value::Object(s)) = (&mut cur, set) {
            for (k, v) in s {
                if !c.contains_key(k) {
                    return Some(Err(format!("unknown Camera Raw setting {k}")));
                }
                c.insert(k.clone(), v.clone());
            }
        }
        next_params = match serde_json::from_value::<CameraRaw>(cur) {
            Ok(p) => p,
            Err(e) => return Some(Err(format!("bad Camera Raw settings: {e}"))),
        };
        if let Err(e) = next_params.validate_changes(&d.original) {
            return Some(Err(format!("bad Camera Raw settings: {e}")));
        }
        use photocraft_algo::camera_raw::{UPRIGHT_MODES, WHITE_BALANCE_MODES};
        if !UPRIGHT_MODES.contains(&next_params.upright.as_str()) || !WHITE_BALANCE_MODES.contains(&next_params.white_balance.as_str()) {
            return Some(Err(format!(
                "bad Camera Raw settings: upright must be one of {}, whiteBalance one of {}",
                UPRIGHT_MODES.join("|"),
                WHITE_BALANCE_MODES.join("|")
            )));
        }
    }
    let scope_update = match ui.get("scope") {
        Some(scope) => match super::camera_raw_scope_ui::prepare_control(&next_params, d.selection.is_some(), &app.ui.camera_raw_scope, scope) {
            Ok(update) => Some(update),
            Err(e) => return Some(Err(e)),
        },
        None => None,
    };
    let layout = match ui.get("layout") {
        Some(v) => {
            let mut cur = serde_json::to_value(&app.ui.camera_raw_layout).unwrap_or(json!({}));
            let (Value::Object(c), Value::Object(s)) = (&mut cur, v) else {
                return Some(Err("Camera Raw layout must be an object".into()));
            };
            for (k, v) in s {
                if !c.contains_key(k) {
                    return Some(Err(format!("unknown Camera Raw layout property {k}")));
                }
                c.insert(k.clone(), v.clone());
            }
            match serde_json::from_value::<crate::state::CameraRawLayoutState>(cur) {
                Ok(l) if !crate::camera_raw_tools_ui::TOOLS.iter().any(|(id, _, _)| *id == l.tool) => {
                    return Some(Err(format!("unknown Camera Raw tool {}", l.tool)));
                }
                Ok(l) if !l.open.iter().all(|o| crate::camera_raw_panels::PANELS.iter().any(|(p, _)| p == o)) => {
                    return Some(Err("Camera Raw layout.open must list panel ids".into()));
                }
                Ok(l) if !l.groups.iter().all(|g| crate::camera_raw_panels::GROUPS.contains(&g.as_str())) => {
                    return Some(Err("Camera Raw layout.groups must list group ids".into()));
                }
                Ok(l) => Some(l),
                Err(e) => return Some(Err(format!("bad Camera Raw layout: {e}"))),
            }
        }
        None => None,
    };
    let hidden = match ui.get("hidden") {
        Some(v) => match serde_json::from_value::<Vec<String>>(v.clone()) {
            Ok(h) if h.iter().all(|id| crate::camera_raw_panels::PANELS.iter().any(|(p, _)| p == id)) => Some(h),
            _ => return Some(Err("Camera Raw hidden must list panel ids".into())),
        },
        None => None,
    };
    let before_after = match ui.get("beforeAfter") {
        Some(v) => match serde_json::from_value::<BeforeAfter>(v.clone()) {
            Ok(b) => Some(b),
            Err(_) => return Some(Err("Camera Raw beforeAfter must be off, leftRight, leftRightSplit, topBottom or topBottomSplit".into())),
        },
        None => None,
    };
    let navigation = match ui.get("view") {
        Some(view) => match super::camera_raw_preview_ui::prepare(app.ui.camera_raw_preview, view) {
            Ok(next) => Some(next),
            Err(e) => return Some(Err(e)),
        },
        None => None,
    };
    let demosaic = match ui.get("demosaic") {
        None => None,
        Some(_) if d.raw_open.is_none() => return Some(Err("demosaic applies only while a raw file opens".into())),
        Some(v) => match v.as_str().and_then(photocraft_io::raw::Demosaic::from_id) {
            Some(m) => Some(m),
            None => return Some(Err("demosaic must be ahd, mhc or bilinear".into())),
        },
    };
    let d = app.camera_raw.as_mut()?;
    if let (Some(m), Some(raw)) = (demosaic, d.raw_open.as_mut()) {
        raw.demosaic = m;
    }
    if let Some(view) = navigation {
        app.ui.camera_raw_preview = view;
    }
    if d.params.point_curve != next_params.point_curve {
        d.curve_state = Default::default();
    }
    if d.params != next_params {
        remember_edit(d);
        d.dirty = true;
        d.params = next_params;
    }
    if let Some(layout) = layout {
        app.ui.camera_raw_layout = layout;
    }
    if let Some(mut h) = hidden {
        h.dedup();
        d.dirty |= d.hidden != h;
        d.hidden = h;
    }
    if let Some(b) = before_after {
        d.before_after = b;
    }
    if let Some(b) = ui.get("profileBrowser").and_then(Value::as_bool) {
        d.dirty |= b && !d.profile_browser;
        d.profile_browser = b;
    }
    if let Some(b) = ui.get("before").and_then(Value::as_bool) {
        d.show_before = b;
    }
    if let Some(update) = scope_update {
        update.apply(d, &mut app.ui.camera_raw_scope);
    }
    if ui.get("undo").and_then(Value::as_bool) == Some(true) {
        undo_edit(d);
    }
    if ui.get("redo").and_then(Value::as_bool) == Some(true) {
        redo_edit(d);
    }
    if d.dirty {
        d.render(ctx);
    }
    if app.ui.camera_raw_scope.vectorscope
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.ensure_scope(app.ui.camera_raw_scope.selected_region);
    }
    super::camera_raw_scope_ui::persist(app, ctx);
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        cancel(app);
        return Some(Ok(json!({"cancelled": true})));
    }
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        return Some(commit(app));
    }
    Some(Ok(app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)).unwrap_or(Value::Null)))
}

/// Save Settings…: the dialog's settings as a Camera Raw `.xmp` settings file.
fn save_settings(app: &mut PhotocraftApp) -> Result<Value, String> {
    let d = app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    let bytes = photocraft_io::camera_raw_xmp::save(&d.params).into_bytes();
    app.pick_save("Camera Raw Settings.xmp", move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        Ok(json!({"path": path}))
    })
}

/// Load Settings…: applies a Camera Raw `.xmp` settings file's supported settings over the
/// dialog's; the status bar names how many were not supported.
fn load_settings(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    app.pick_file_bytes_filtered(&["xmp"], |app, name, bytes| {
        let file = crate::file_open::display_name(&name);
        let d = app.camera_raw.as_mut().ok_or(tl!("Camera Raw isn't open"))?;
        let mut next = d.params.clone();
        let report = photocraft_io::camera_raw_xmp::load(&mut next, &bytes).map_err(|e| format!("{file}: {e}"))?;
        next.validate_changes(&d.original).map_err(|e| format!("{file}: {e}"))?;
        if d.params.point_curve != next.point_curve {
            d.curve_state = Default::default();
        }
        if d.params != next {
            remember_edit(d);
            d.dirty = true;
            d.params = next;
        }
        app.ui.status_error = false;
        app.ui.status = if report.ignored.is_empty() {
            crate::i18n::fmt(tl!("Loaded Camera Raw settings from {file}"), &[("file", &file)])
        } else {
            let template = "Loaded Camera Raw settings from {file} ({n} not supported)";
            crate::i18n::fmt(tl!(template), &[("file", &file), ("n", &report.ignored.len().to_string())])
        };
        Ok(json!({"applied": report.applied, "ignored": report.ignored}))
    })
}

fn report_file_error(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status_error = true;
        app.ui.status = e;
    }
}

const RAW_UNDO_LIMIT: usize = 40;

fn remember_edit(d: &mut CameraRawDialog) {
    if d.undo.len() == RAW_UNDO_LIMIT {
        d.undo.remove(0);
    }
    d.undo.push(d.params.clone());
    d.redo.clear();
}

fn undo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(prev) = d.undo.pop() else { return false };
    d.redo.push(d.params.clone());
    if d.params.point_curve != prev.point_curve {
        d.curve_state = Default::default();
    }
    d.params = prev;
    d.dirty = true;
    d.edit_origin = None;
    true
}

fn redo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(next) = d.redo.pop() else { return false };
    d.undo.push(d.params.clone());
    if d.params.point_curve != next.point_curve {
        d.curve_state = Default::default();
    }
    d.params = next;
    d.dirty = true;
    d.edit_origin = None;
    true
}

fn history_keys(d: &mut CameraRawDialog, ctx: &egui::Context) {
    let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    if ctx.input_mut(|i| i.consume_key(cmd_shift, egui::Key::Z)) {
        redo_edit(d);
    } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
        undo_edit(d);
    }
}

fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    let d = app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    let previous = d.command_params();
    let result = match d.target {
        Target::NewFilter => {
            let mut p = d.command_params();
            p["layer"] = json!(d.layer.0);
            app.run("filter.cameraRaw", p)?
        }
        Target::SmartFilter(index) => {
            d.params.validate_changes(&d.original)?;
            // Every setting, not just the non-defaults: setParams merges into the stored ones,
            // so a control reset to its default must overwrite the old value.
            let mut params = serde_json::to_value(&d.params).map_err(|e| e.to_string())?;
            if let Value::Object(m) = &mut params {
                m.remove("pixelScale");
            }
            app.run("layer.smartFilter.setParams", json!({"layer": d.layer.0, "index": index, "params": params}))?
        }
        Target::OpenRaw => {
            let raw = d.raw_open.clone().ok_or("no raw file is being opened")?;
            let params = d.params.clone();
            commit_open_raw(app, &params, &raw)?
        }
    };
    app.camera_raw = None;
    // More › Apply Previous Settings.
    app.session.prefs.edit(|p| p.dialogs.insert(PREVIOUS_KEY.into(), previous));
    Ok(result)
}

/// Preference keys: the panel layout, and the settings of the last OK.
const LAYOUT_KEY: &str = "filter.cameraRaw.layout";
const PREVIOUS_KEY: &str = "filter.cameraRaw.previous";

/// The open-time demosaic choice (in Detail). Photoshop's Camera Raw picks one method for every
/// file; here it is the user's. Open re-develops with it; the preview keeps the method the file
/// was opened with (the differences are finer than the preview's pixels).
pub(crate) fn demosaic_row(ui: &mut egui::Ui, m: &mut photocraft_io::raw::Demosaic) {
    use photocraft_io::raw::Demosaic;
    let c = crate::camera_raw_style::Acr::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(crate::camera_raw_style::INSET_L);
        ui.set_min_height(28.0);
        ui.label(egui::RichText::new(tl!("Demosaic")).font(crate::camera_raw_style::label_font()).color(c.label)).on_hover_text(tl!(
            "How the camera's colour filter mosaic is filled in. It is applied when the file opens; the preview shows the method it was opened with."
        ));
        ui.add_space(6.0);
        let options = [
            (Demosaic::Ahd, "AHD (adaptive homogeneity)", "Hirakawa–Parks: the cleanest edges and the least colour fringing, the slowest."),
            (Demosaic::Mhc, "Malvar–He–Cutler", "Gradient-corrected interpolation: sharp and fast, with some colour fringing on fine detail."),
            (Demosaic::Bilinear, "Bilinear", "The fastest and the softest, with colour fringing on fine detail."),
        ];
        widgets::dropdown_with_tooltips(ui, "camera-raw-demosaic", m, &options, 160.0);
    });
}

/// True for an identity point curve (empty or on the diagonal).
pub(crate) fn is_linear(c: &[[f32; 2]]) -> bool {
    c.len() < 2 || c.iter().all(|p| (p[0] - p[1]).abs() < 1e-3)
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.camera_raw.is_none()
        && let Some(raw) = app.pending_raw_open.take()
        && let Err(e) = open_raw(app, ctx, raw)
    {
        // The document stays open, developed with the defaults.
        app.ui.status = e;
        app.ui.status_error = true;
    }
    let Some(d) = app.camera_raw.as_ref() else { return };
    // Where the platform has no extra windows (the web build), draw over the main window.
    if ctx.embed_viewports() {
        draw(app, ctx, false);
        return;
    }
    let window_title = d.window_title();
    // Camera Raw is modal, as in Photoshop: the main window is dimmed and takes no input while it
    // is open (in a separate window, or drawn over it where there are no extra windows).
    let main = ctx.content_rect();
    egui::Area::new(egui::Id::new("camera-raw-modal")).order(egui::Order::Foreground).fixed_pos(main.min).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(main.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, 0.0, Tokens::get(ui.ctx()).scrim);
    });
    // A window of its own (title bar, moved and resized like any window), like Adobe Camera Raw.
    let builder = egui::ViewportBuilder::default().with_title(window_title).with_inner_size([1632.0, 1046.0]).with_min_inner_size([960.0, 640.0]);
    let mut close = false;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("camera-raw-window"), builder, |ui, class| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            close = true;
        }
        draw(app, ui.ctx(), class != egui::ViewportClass::EmbeddedWindow);
    });
    if close && app.camera_raw.is_some() {
        cancel(app);
    }
}

impl CameraRawDialog {
    /// "Camera Raw (name)" for the open-time dialog, "Camera Raw Filter (layer)" otherwise.
    fn window_title(&self) -> String {
        if self.target == Target::OpenRaw {
            crate::i18n::fmt(tl!("Camera Raw ({name})"), &[("name", &self.layer_name)])
        } else {
            crate::i18n::fmt(tl!("Camera Raw Filter ({layer})"), &[("layer", &self.layer_name)])
        }
    }
}

/// The dialog's contents over the whole of `ctx`'s window. `own_window`: the OS window already
/// shows the title; Camera Raw's own top bar (file name, Save, Preferences, Full Screen) is drawn
/// either way.
fn draw(app: &mut PhotocraftApp, ctx: &egui::Context, own_window: bool) {
    let t = Tokens::get(ctx);
    let c = crate::camera_raw_style::Acr::get(ctx);
    let screen = ctx.content_rect();
    let current_display = app.session.active().and_then(|st| app.session.color.canvas_display(&st.doc).ok());
    let mut action: Option<&str> = None;
    let previous: Option<CameraRaw> = app.session.prefs().dialogs.get(PREVIOUS_KEY).and_then(|v| serde_json::from_value(v.clone()).ok());
    egui::Area::new(egui::Id::new("camera-raw-dialog")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let Some(d) = app.camera_raw.as_mut() else { return };
        history_keys(d, ctx);
        let pointer = ctx.input(|i| i.pointer.primary_down());
        if pointer && d.edit_origin.is_none() {
            d.edit_origin = Some(d.params.clone());
        }
        if let Some(display) = &current_display
            && display.key != d.display.key
        {
            d.display = display.clone();
            d.dirty = true;
        }
        d.curve_rect = None;
        d.scope.vectorscope_rect = None;
        if d.dirty {
            d.render(ctx);
        }
        let (full, _) = ui.allocate_exact_size(screen.size(), Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, c.base);
        let layout = &mut app.ui.camera_raw_layout;
        let scope = &mut app.ui.camera_raw_scope;
        // Columns: preview | rule | panels (+ scroll bar) | rule | tool strip.
        let strip = ERect::from_min_max(pos2(full.right() - STRIP_W, full.top() + TOP_H), pos2(full.right(), full.bottom() - FOOTER_H));
        let panels = ERect::from_min_max(pos2(strip.left() - 1.0 - PANEL_W, strip.top()), pos2(strip.left() - 1.0, strip.bottom()));
        let preview_body = ERect::from_min_max(pos2(full.left(), strip.top()), pos2((panels.left() - 2.0).max(full.left() + 1.0), strip.bottom()));
        // Top bar.
        let top = ERect::from_min_max(full.min, pos2(full.right(), full.top() + TOP_H));
        let _ = own_window;
        let title = if d.target == Target::OpenRaw { d.layer_name.clone() } else { d.doc_name.clone() };
        painter.text(pos2(preview_body.center().x, top.center().y), Align2::CENTER_CENTER, title, FontId::proportional(13.0), c.text);
        let mut top_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(ERect::from_min_max(pos2(panels.left(), top.top()), pos2(top.right() - 4.0, top.bottom())))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        top_ui.spacing_mut().item_spacing.x = 2.0;
        let fs = crate::icons::button(&mut top_ui, "maximize-2", 32.0, false, tl!("Toggle Full Screen mode (F)"));
        if fs.clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!ctx.input(|i| i.viewport().maximized.unwrap_or(false))));
        }
        top_ui.add_space(4.0);
        let (sep, _) = top_ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
        top_ui.painter().rect_filled(sep, 0.0, c.rule);
        top_ui.add_space(4.0);
        if crate::icons::button(&mut top_ui, "settings", 32.0, d.prefs_open, tl!("Open preferences dialog (Ctrl+K)")).clicked() {
            d.prefs_open = true;
        }
        top_ui
            .add_enabled(false, egui::Button::image(crate::icons::image("download", 18.0, c.faint)).frame(false))
            .on_disabled_hover_text(tl!("Convert and save image: not available in the Camera Raw Filter"));
        // Preview.
        let bar = ERect::from_min_max(pos2(preview_body.left(), preview_body.bottom() - PREVIEW_BAR_H), preview_body.max);
        let well = ERect::from_min_max(preview_body.min, pos2(preview_body.right(), bar.top()));
        painter.rect_filled(well, 0.0, c.well);
        painter.rect_filled(ERect::from_min_max(pos2(well.right(), strip.top()), pos2(panels.left(), strip.bottom())), 0.0, c.rule);
        let view = well.shrink(8.0);
        d.viewport = Some(view);
        d.preview_ppp = ctx.pixels_per_point();
        let size = vec2(d.full_w as f32, d.full_h as f32) / ctx.pixels_per_point();
        let navigation = &mut app.ui.camera_raw_preview;
        let mut preview = ui.new_child(egui::UiBuilder::new().max_rect(view));
        preview.set_clip_rect(view.intersect(ui.clip_rect()));
        let mode = d.before_after;
        // Side-by-side views split the well; the After view takes the navigation and the scopes.
        let (before_view, after_view) = match mode {
            BeforeAfter::LeftRight => {
                let w = (view.width() - 4.0) / 2.0;
                (Some(ERect::from_min_size(view.min, vec2(w, view.height()))), ERect::from_min_max(pos2(view.right() - w, view.top()), view.max))
            }
            BeforeAfter::TopBottom => {
                let h = (view.height() - 4.0) / 2.0;
                (Some(ERect::from_min_size(view.min, vec2(view.width(), h))), ERect::from_min_max(pos2(view.left(), view.bottom() - h), view.max))
            }
            _ => (None, view),
        };
        let (r, response, panning, zoom_box) = super::camera_raw_preview_ui::interact(&mut preview, navigation, after_view, size, &mut scope.sampler_tool);
        let shown = crate::camera_raw_panels::visible(&d.params, &d.hidden);
        let tex = if d.show_before && mode == BeforeAfter::Off { d.before_tex.as_ref() } else { d.tex.as_ref() };
        let uv = ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        if let Some(tex) = tex {
            widgets::checker(preview.painter(), r.intersect(after_view), 8.0);
            preview.painter().with_clip_rect(after_view).image(tex.id(), r, uv, Color32::WHITE);
            if mode == BeforeAfter::Off {
                // Raw-stage sliders included: the worker retones the full frame as Open does.
                let detail_needed = r.width() * ctx.pixels_per_point() > d.pw as f32 || r.height() * ctx.pixels_per_point() > d.ph as f32;
                d.detail.paint(&preview, r, d.preview_revision, d.show_before, &shown, detail_needed, &d.display);
                if d.detail.pending() {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
            d.scope.preview_rect = Some(r);
            super::camera_raw_scope_ui::preview(&mut preview, d, scope, r, &response, panning);
        }
        if let Some(before) = d.before_tex.as_ref() {
            match (mode, before_view) {
                (_, Some(bv)) => {
                    let br = navigation.image_rect(bv, size);
                    widgets::checker(preview.painter(), br.intersect(bv), 8.0);
                    preview.painter().with_clip_rect(bv).image(before.id(), br, uv, Color32::WHITE);
                    if mode == BeforeAfter::LeftRight {
                        preview.painter().vline(view.center().x, view.y_range(), Stroke::new(1.0, Color32::from_black_alpha(200)));
                        split_labels(&preview, view, true, &c);
                    } else {
                        view_label(&preview, bv, tl!("Before"), &c);
                        view_label(&preview, after_view, tl!("After"), &c);
                    }
                }
                (BeforeAfter::LeftRightSplit, None) => {
                    let half = ERect::from_min_max(view.min, pos2(view.center().x, view.bottom()));
                    preview.painter().with_clip_rect(half).image(before.id(), r, uv, Color32::WHITE);
                    preview.painter().vline(view.center().x, view.y_range(), Stroke::new(1.0, Color32::from_black_alpha(200)));
                    split_labels(&preview, view, true, &c);
                }
                (BeforeAfter::TopBottomSplit, None) => {
                    let half = ERect::from_min_max(view.min, pos2(view.right(), view.center().y));
                    preview.painter().with_clip_rect(half).image(before.id(), r, uv, Color32::WHITE);
                    preview.painter().hline(view.x_range(), view.center().y, Stroke::new(1.0, Color32::from_black_alpha(200)));
                    split_labels(&preview, view, false, &c);
                }
                _ => {}
            }
        }
        if layout.grid {
            let g = r.intersect(after_view);
            for i in 1..8 {
                let f = i as f32 / 8.0;
                let s = Stroke::new(1.0, Color32::from_white_alpha(if i % 4 == 0 { 70 } else { 35 }));
                preview.painter().vline(r.left() + f * r.width(), g.y_range(), s);
                preview.painter().hline(g.x_range(), r.top() + f * r.height(), s);
            }
        }
        // White Balance tool: a click neutralises the colour under the pointer.
        if d.wb_picker {
            response.clone().on_hover_cursor(egui::CursorIcon::Crosshair);
            if response.clicked()
                && let Some(p) = response.interact_pointer_pos()
                && r.contains(p)
            {
                let u = ((p - r.min) / r.size()).clamp(vec2(0.0, 0.0), vec2(1.0, 1.0));
                d.pick_white_balance([u.x, u.y]);
            }
        }
        if let Some(region) = zoom_box {
            preview.painter().rect_stroke(region, 0.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
        preview_bar(ui, d, navigation, bar, after_view, size, &c);
        // Right column: histogram (and the vectorscope when shown), the info row, the tool.
        let mut head = ui.new_child(egui::UiBuilder::new().max_rect(ERect::from_min_max(panels.min, pos2(panels.right(), panels.top() + HISTOGRAM_H + 300.0))));
        head.set_clip_rect(panels.intersect(ui.clip_rect()));
        head.spacing_mut().item_spacing.y = 0.0;
        super::camera_raw_scope_ui::header(&mut head, d, scope, HISTOGRAM_H);
        let (info, _) = head.allocate_exact_size(vec2(panels.width(), INFO_H), Sense::hover());
        if let Some(raw) = &d.raw_open {
            head.painter().text(info.center(), Align2::CENTER_CENTER, &raw.name, FontId::proportional(11.0), c.label);
        }
        let used = head.min_rect().bottom().max(info.bottom());
        painter.rect_filled(ERect::from_min_max(pos2(panels.left(), used), pos2(panels.right(), used + 2.0)), 0.0, c.rule);
        let list = ERect::from_min_max(pos2(panels.left(), used + 2.0), panels.max);
        let mut props = ui.new_child(egui::UiBuilder::new().max_rect(list));
        props.set_clip_rect(list.intersect(ui.clip_rect()));
        d.scope.alt_tone = None;
        let mut dirty = false;
        // Camera Raw keeps its scroll bar's track visible.
        egui::ScrollArea::vertical()
            .id_salt(("camera-raw-props", layout.tool.clone()))
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(&mut props, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.set_width(list.width() - 15.0);
                match layout.tool.as_str() {
                    "remove" => crate::camera_raw_tools_ui::remove(ui, &mut d.tools),
                    "masking" => crate::camera_raw_tools_ui::masking(ui),
                    "redEye" => crate::camera_raw_tools_ui::red_eye(ui, &mut d.tools),
                    "presets" => dirty |= crate::camera_raw_tools_ui::presets(ui, d),
                    _ => dirty |= crate::camera_raw_panels::edit(ui, d, layout),
                }
            });
        // Tool strip.
        painter.rect_filled(ERect::from_min_max(pos2(strip.left() - 1.0, strip.top()), pos2(strip.left(), strip.bottom())), 0.0, c.rule);
        let mut tools = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(4.0, 6.0))).layout(egui::Layout::top_down(egui::Align::Center)));
        tools.spacing_mut().item_spacing.y = 2.0;
        for (id, icon, tip) in crate::camera_raw_tools_ui::TOOLS {
            let selected = layout.tool == id;
            let r = crate::icons::button(&mut tools, icon, 32.0, selected, tl!(tip));
            if id == "edit" && !d.params.is_identity() {
                tools.painter().circle_filled(r.rect.right_top() + vec2(-3.0, 5.0), 2.5, c.text);
            }
            if r.clicked() {
                layout.tool = id.to_owned();
            }
        }
        let more = crate::icons::button(&mut tools, "ellipsis", 32.0, false, tl!("More image settings"));
        egui::Popup::menu(&more).show(|ui| {
            dirty |= crate::camera_raw_tools_ui::more_menu(ui, d, previous.clone(), &mut action);
        });
        let mut low = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(4.0, 6.0))).layout(egui::Layout::bottom_up(egui::Align::Center)));
        low.spacing_mut().item_spacing.y = 2.0;
        if crate::icons::button(&mut low, "grid-3x3", 32.0, layout.grid, tl!("Toggle grid overlay (Ctrl+Shift+G)")).clicked() {
            layout.grid = !layout.grid;
        }
        if crate::icons::button(&mut low, "pipette", 32.0, scope.sampler_tool, tl!("Toggle sampler overlay (S)")).clicked() {
            scope.sampler_tool = !scope.sampler_tool;
        }
        low.add_space(6.0);
        let hand = crate::icons::button(&mut low, "hand", 32.0, navigation.hand && !scope.sampler_tool, tl!("Hand Tool (H)"));
        if hand.double_clicked() {
            navigation.fit();
        } else if hand.clicked() {
            navigation.hand = true;
            scope.sampler_tool = false;
        }
        let zoom = crate::icons::button(&mut low, "zoom-in", 32.0, !navigation.hand && !scope.sampler_tool, tl!("Zoom Tool (Z)"));
        if zoom.double_clicked() {
            navigation.zoom_at(1.0, after_view.center(), after_view, size);
        } else if zoom.clicked() {
            navigation.hand = false;
            scope.sampler_tool = false;
        }
        super::camera_raw_scope_ui::floating(ctx, d, scope, panels);
        super::camera_raw_scope_ui::shortcuts(ctx, scope);
        crate::camera_raw_tools_ui::preferences(ctx, &mut d.prefs_open, &mut d.prefs_tab, layout);
        d.dirty |= dirty;
        if d.dirty {
            ctx.request_repaint();
        }
        // Footer: OK (Open for a raw being opened) and Cancel.
        let foot = ERect::from_min_max(pos2(full.left(), full.bottom() - FOOTER_H), full.max);
        let ok_label = if d.target == Target::OpenRaw { tl!("Open") } else { tl!("OK") };
        let cancel_rect = ERect::from_min_size(pos2(foot.right() - 80.0, foot.center().y - 12.0), vec2(60.0, 24.0));
        let ok_rect = ERect::from_min_size(pos2(cancel_rect.left() - 16.0 - 70.0, foot.center().y - 12.0), vec2(70.0, 24.0));
        let ok = ui.interact(ok_rect, ui.id().with("cr-ok"), Sense::click());
        ok.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, ok_label));
        painter.rect_filled(ok_rect, 12.0, if ok.hovered() { c.hover } else { c.base });
        painter.rect_stroke(ok_rect, 12.0, Stroke::new(1.2, c.text), egui::StrokeKind::Inside);
        painter.text(ok_rect.center(), Align2::CENTER_CENTER, ok_label, FontId::proportional(12.5), c.text);
        let cancel = ui.interact(cancel_rect, ui.id().with("cr-cancel"), Sense::click());
        cancel.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Cancel")));
        if cancel.hovered() {
            painter.rect_filled(cancel_rect, 12.0, c.hover);
        }
        painter.text(cancel_rect.center(), Align2::CENTER_CENTER, tl!("Cancel"), FontId::proportional(12.5), c.text);
        if ok.clicked() {
            action = Some("ok");
        }
        if cancel.clicked() {
            action = Some("cancel");
        }
        if !ctx.input(|i| i.pointer.primary_down())
            && let Some(origin) = d.edit_origin.take()
            && origin != d.params
        {
            let edited = std::mem::replace(&mut d.params, origin);
            remember_edit(d);
            d.params = edited;
        }
    });
    super::camera_raw_scope_ui::persist(app, ctx);
    persist_layout(app, ctx);
    shortcuts(app, ctx, &mut action);
    match action {
        Some("ok") => {
            if let Err(e) = commit(app) {
                app.ui.status = e;
            }
        }
        Some("cancel") => cancel(app),
        Some("save") => {
            let r = save_settings(app);
            report_file_error(app, r);
        }
        Some("load") => {
            let r = load_settings(app);
            report_file_error(app, r);
        }
        _ => {}
    }
}

/// "Before" / "After" over a side-by-side view.
fn view_label(ui: &egui::Ui, view: ERect, text: &str, c: &crate::camera_raw_style::Acr) {
    let g = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(11.0), c.text);
    let r = ERect::from_center_size(pos2(view.center().x, view.top() + 10.0), g.size() + vec2(14.0, 6.0));
    ui.painter().rect_filled(r, 0.0, c.rule.gamma_multiply(0.9));
    ui.painter().galley(r.center() - g.size() * 0.5, g, c.text);
}

/// The two labels of a split (or left/right) view: "Before" | "After" side by side at the
/// split, as Camera Raw tabs them; above each half for a top/bottom split.
fn split_labels(ui: &egui::Ui, view: ERect, vertical: bool, c: &crate::camera_raw_style::Acr) {
    if !vertical {
        view_label(ui, ERect::from_min_max(view.min, pos2(view.right(), view.center().y)), tl!("Before"), c);
        view_label(ui, ERect::from_min_max(pos2(view.left(), view.center().y), view.max), tl!("After"), c);
        return;
    }
    let font = FontId::proportional(11.0);
    for (text, left) in [(tl!("Before"), true), (tl!("After"), false)] {
        let g = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), c.text);
        let size = g.size() + vec2(14.0, 6.0);
        let x = if left { view.center().x - size.x - 1.0 } else { view.center().x + 1.0 };
        let r = ERect::from_min_size(pos2(x, view.top()), size);
        ui.painter().rect_filled(r, 0.0, c.rule.gamma_multiply(0.9));
        ui.painter().galley(r.center() - g.size() * 0.5, g, c.text);
    }
}

/// The bar under the preview: Fit (n %) | 100 % | zoom menu on the left; Before/After controls
/// on the right.
fn preview_bar(
    ui: &mut egui::Ui,
    d: &mut CameraRawDialog,
    navigation: &mut crate::state::CameraRawPreviewState,
    bar: ERect,
    view: ERect,
    size: egui::Vec2,
    c: &crate::camera_raw_style::Acr,
) {
    let p = ui.painter().clone();
    let fit = navigation.zoom.is_none();
    let scale = navigation.scale(view, size) * 100.0;
    let fit_text = crate::i18n::fmt(tl!("Fit ({percent})"), &[("percent", &format!("{scale:.1}%"))]);
    let mut x = bar.left();
    for (label, selected, id) in [(fit_text.as_str(), fit, "fit"), ("100%", navigation.zoom == Some(1.0), "100")] {
        let g = p.layout_no_wrap(label.to_owned(), FontId::proportional(12.0), c.text);
        let r = ERect::from_min_size(pos2(x, bar.top()), vec2(g.size().x + 22.0, bar.height()));
        let resp = ui.interact(r, ui.id().with(("cr-zoom-tab", id)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label));
        if resp.hovered() {
            p.rect_filled(r, 0.0, c.hover.gamma_multiply(0.4));
        }
        p.galley(r.center() - g.size() * 0.5, g, c.text);
        if selected {
            p.rect_filled(ERect::from_min_max(pos2(r.left(), r.bottom() - 2.0), r.max), 0.0, c.text);
        }
        if resp.clicked() {
            if id == "fit" {
                navigation.fit();
            } else {
                navigation.zoom_at(1.0, view.center(), view, size);
            }
        }
        x = r.right() + 1.0;
    }
    let menu_rect = ERect::from_min_size(pos2(x, bar.top()), vec2(22.0, bar.height()));
    let menu = ui.interact(menu_rect, ui.id().with("cr-zoom-menu"), Sense::click()).on_hover_text(tl!("Select zoom level"));
    widgets::chevron_icon(ui, ERect::from_center_size(menu_rect.center(), vec2(12.0, 12.0)), &ui.style().visuals.widgets.inactive, false);
    egui::Popup::menu(&menu).show(|ui| {
        for (z, label) in [(0.06, "6%"), (0.12, "12%"), (0.25, "25%"), (0.33, "33%"), (0.5, "50%"), (0.66, "66%")] {
            if ui.button(label).clicked() {
                navigation.zoom_at(z, view.center(), view, size);
            }
        }
        ui.separator();
        if ui.add(egui::Button::new("100%").shortcut_text("Ctrl+Alt+0")).clicked() {
            navigation.zoom_at(1.0, view.center(), view, size);
        }
        ui.separator();
        for (z, label) in [(2.0, "200%"), (3.0, "300%"), (4.0, "400%"), (8.0, "800%"), (16.0, "1600%")] {
            if ui.button(label).clicked() {
                navigation.zoom_at(z, view.center(), view, size);
            }
        }
        ui.separator();
        if ui.add(egui::Button::new(tl!("Fit in View")).selected(navigation.zoom.is_none()).shortcut_text("Ctrl+0")).clicked() {
            navigation.fit();
        }
    });
    let mut right = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(12.0, 4.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    right.spacing_mut().item_spacing.x = 2.0;
    let ba_icon = if matches!(d.before_after, BeforeAfter::TopBottom | BeforeAfter::TopBottomSplit) { "rows-2" } else { "columns-2" };
    let ba = crate::icons::button(&mut right, ba_icon, 30.0, d.before_after != BeforeAfter::Off, tl!("Cycle between Before/After views (Q)"));
    if ba.clicked() {
        d.before_after = d.before_after.next();
    }
    ba.context_menu(|ui| {
        for (mode, label) in [
            (BeforeAfter::LeftRight, tl!("Before/After Left/Right")),
            (BeforeAfter::LeftRightSplit, tl!("Before/After Left/Right Split")),
            (BeforeAfter::TopBottom, tl!("Before/After Top/Bottom")),
            (BeforeAfter::TopBottomSplit, tl!("Before/After Top/Bottom Split")),
        ] {
            if ui.add(egui::Button::new(label).selected(d.before_after == mode)).clicked() {
                d.before_after = if d.before_after == mode { BeforeAfter::Off } else { mode };
                ui.close();
            }
        }
    });
    right.add_space(4.0);
    let (sep, _) = right.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
    right.painter().rect_filled(sep, 0.0, c.rule);
    right.add_space(4.0);
    if d.before_after == BeforeAfter::Off {
        let before = crate::icons::button(&mut right, "square", 30.0, d.show_before, tl!("Toggle between current settings and defaults (P)"));
        if before.clicked() {
            d.show_before = !d.show_before;
        }
    } else {
        // Right to left: Copy Before to After, Swap, Copy After to Before (Camera Raw's order
        // reads ⇤ ⇄ ⇥ from the left).
        if crate::icons::button(&mut right, "chevrons-right", 30.0, false, tl!("Copy Before settings to After (Alt+Shift+P)")).clicked() {
            d.params = CameraRaw { pixel_scale: d.params.pixel_scale, encoding_gamma: d.params.encoding_gamma, ..d.before_params.clone() };
            d.dirty = true;
        }
        if crate::icons::button(&mut right, "arrow-left-right", 30.0, false, tl!("Swap Before and After settings (Ctrl+Alt+Shift+P)")).clicked() {
            let after = d.params.clone();
            d.params = CameraRaw { pixel_scale: after.pixel_scale, encoding_gamma: after.encoding_gamma, ..d.before_params.clone() };
            d.before_params = after;
            (d.dirty, d.before_dirty) = (true, true);
        }
        if crate::icons::button(&mut right, "chevrons-left", 30.0, false, tl!("Copy After settings to Before (Ctrl+Alt+P)")).clicked() {
            d.before_params = d.params.clone();
            (d.dirty, d.before_dirty) = (true, true);
        }
    }
    if d.detail.pending() {
        right.spinner();
    }
    if let Some(error) = &d.detail.error {
        right.label(egui::RichText::new(tl!("Full-resolution preview unavailable")).color(c.faint)).on_hover_text(error);
    }
}

/// Camera Raw's keys (not while a text field has focus): Q cycles Before/After, Y shows Left /
/// Right, P (or \) toggles the original, V B&W, E B M Shift+E Shift+P pick the tool, Ctrl+U Auto,
/// Ctrl+K Preferences, Esc cancels and Enter is OK.
fn shortcuts(app: &mut PhotocraftApp, ctx: &egui::Context, action: &mut Option<&str>) {
    let Some(d) = app.camera_raw.as_mut() else { return };
    let focused = ctx.text_edit_focused();
    let ended_edit = std::mem::replace(&mut d.text_focus_prev, focused);
    if focused || ended_edit {
        return;
    }
    use egui::{Key, Modifiers};
    let layout = &mut app.ui.camera_raw_layout;
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Q) {
            d.before_after = d.before_after.next();
        }
        if i.consume_key(Modifiers::NONE, Key::Y) {
            d.before_after = if d.before_after == BeforeAfter::LeftRight { BeforeAfter::Off } else { BeforeAfter::LeftRight };
        }
        if i.consume_key(Modifiers::NONE, Key::P) || i.consume_key(Modifiers::NONE, Key::Backslash) {
            d.show_before = !d.show_before;
        }
        // Keys change the settings without a pointer drag: each is an undo step of its own.
        if i.consume_key(Modifiers::NONE, Key::V) {
            remember_edit(d);
            d.params.monochrome = !d.params.monochrome;
            d.dirty = true;
        }
        if i.consume_key(Modifiers::COMMAND, Key::U) {
            let was = d.params.clone();
            d.auto_tone();
            if d.params != was {
                let toned = std::mem::replace(&mut d.params, was);
                remember_edit(d);
                d.params = toned;
            }
            d.dirty = true;
        }
        if i.consume_key(Modifiers::COMMAND, Key::K) {
            d.prefs_open = true;
        }
        if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::G) {
            layout.grid = !layout.grid;
        }
        for (key, mods, tool) in [
            (Key::E, Modifiers::NONE, "edit"),
            (Key::B, Modifiers::NONE, "remove"),
            (Key::M, Modifiers::NONE, "masking"),
            (Key::E, Modifiers::SHIFT, "redEye"),
            (Key::P, Modifiers::SHIFT, "presets"),
        ] {
            if i.consume_key(mods, key) {
                layout.tool = tool.into();
            }
        }
        if i.consume_key(Modifiers::NONE, Key::I) {
            d.wb_picker = !d.wb_picker;
        }
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            if d.wb_picker {
                d.wb_picker = false;
            } else if d.profile_browser {
                d.profile_browser = false;
            } else {
                *action = Some("cancel");
            }
        }
        if i.consume_key(Modifiers::NONE, Key::Enter) && !d.prefs_open {
            *action = Some("ok");
        }
    });
}

/// Remember the panel layout through the preferences store, as the scope view is.
fn persist_layout(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let Ok(value) = serde_json::to_value(&app.ui.camera_raw_layout) else { return };
    if app.session.prefs().dialogs.get(LAYOUT_KEY) != Some(&value) {
        app.session.prefs.edit(|p| p.dialogs.insert(LAYOUT_KEY.into(), value));
    }
}

/// Linear-light white balance for a colour `rgb` (display encoded): Temperature and Tint that
/// make it neutral, inverting [`photocraft_algo::camera_raw::white_balance_gains`].
pub(crate) fn neutralising_white_balance(rgb: [f32; 3]) -> Option<(f32, f32)> {
    let lin = rgb.map(photocraft_color::convert::srgb_to_linear);
    let [r, g, b] = lin;
    if !(r > 1e-4 && g > 1e-4 && b > 1e-4) {
        return None;
    }
    // Gains ∝ 1/channel; with Camera Raw's slider gains [1 + 0.35 t, 1 − 0.25 n, 1 − 0.35 t]
    // (up to a common factor), k = 1 − 0.25 n follows from the red and blue ratios.
    let k = 2.0 / (g * (1.0 / r + 1.0 / b));
    let t = (k * g / r - 1.0) / 0.35 * 100.0;
    let n = (1.0 - k) / 0.25 * 100.0;
    (t.is_finite() && n.is_finite()).then(|| (t.clamp(-100.0, 100.0).round(), n.clamp(-100.0, 100.0).round()))
}

impl CameraRawDialog {
    /// White Balance tool: neutralise the original colour at `uv` (0..1 over the image).
    pub(crate) fn pick_white_balance(&mut self, uv: [f32; 2]) {
        let x = ((uv[0] * self.pw as f32) as usize).min(self.pw.saturating_sub(1));
        let y = ((uv[1] * self.ph as f32) as usize).min(self.ph.saturating_sub(1));
        // Average a small block of the original so noise does not decide the balance.
        let mut acc = [0.0f32; 3];
        let mut n = 0.0;
        for yy in y.saturating_sub(1)..=(y + 1).min(self.ph.saturating_sub(1)) {
            for xx in x.saturating_sub(1)..=(x + 1).min(self.pw.saturating_sub(1)) {
                if let Some(q) = self.proxy.get(yy * self.pw + xx) {
                    for c in 0..3 {
                        acc[c] += q[c];
                    }
                    n += 1.0;
                }
            }
        }
        if n > 0.0
            && let Some((t, tint)) = neutralising_white_balance(acc.map(|v| v / n))
        {
            self.params.temperature = t;
            self.params.tint = tint;
            self.params.white_balance = "custom".into();
            self.dirty = true;
        }
    }

    /// White balance › Auto: the gray-world balance of the original (mean colour made neutral).
    pub(crate) fn auto_white_balance(&mut self) {
        let mut acc = [0.0f64; 3];
        for q in &self.proxy {
            for c in 0..3 {
                acc[c] += photocraft_color::convert::srgb_to_linear(q[c].clamp(0.0, 1.0)) as f64;
            }
        }
        let n = self.proxy.len().max(1) as f64;
        let mean = acc.map(|v| photocraft_color::convert::linear_to_srgb((v / n) as f32));
        if let Some((t, tint)) = neutralising_white_balance(mean) {
            self.params.temperature = t;
            self.params.tint = tint;
        }
        self.params.white_balance = "auto".into();
        self.dirty = true;
    }

    /// Edit › Auto: textbook auto tone from the original's luminance histogram (median toward
    /// middle gray, whites and blacks to the ends, contrast for flat images). Camera Raw's own
    /// Auto is a trained model; this is a plain starting point.
    pub(crate) fn auto_tone(&mut self) {
        let mut l: Vec<f32> = self.proxy.iter().map(|q| (0.2126 * q[0] + 0.7152 * q[1] + 0.0722 * q[2]).clamp(0.0, 1.0)).filter(|v| v.is_finite()).collect();
        if l.is_empty() {
            return;
        }
        l.sort_by(f32::total_cmp);
        let at = |f: f32| l.get(((l.len() - 1) as f32 * f) as usize).copied().unwrap_or(0.5);
        let (lo, med, hi) = (at(0.005), at(0.5), at(0.995));
        let mean = l.iter().sum::<f32>() / l.len() as f32;
        let sd = (l.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / l.len() as f32).sqrt();
        let lin = photocraft_color::convert::srgb_to_linear;
        let exposure = ((lin(0.45) / lin(med.max(0.02))).log2() * 0.7).clamp(-2.0, 2.0);
        let p = &mut self.params;
        p.exposure = (exposure * 20.0).round() / 20.0;
        p.whites = if hi < 0.97 { ((0.97 - hi) * 220.0).clamp(0.0, 50.0).round() } else { 0.0 };
        p.blacks = if lo > 0.03 { (-(lo - 0.03) * 300.0).clamp(-50.0, 0.0).round() } else { 0.0 };
        p.contrast = if sd < 0.2 { ((0.2 - sd) * 150.0).clamp(0.0, 25.0).round() } else { 0.0 };
        p.highlights = if hi > 0.98 { -30.0 } else { 0.0 };
        p.shadows = if at(0.1) < 0.08 { 20.0 } else { 0.0 };
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_texture_uses_linear_document_display_encoding() {
        let mut doc = photocraft_doc::Document::new("test", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F32);
        doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        let mut color = photocraft_engine::color_cmds::ColorState::default();
        color.settings.monitor_profile = "srgb".into();
        let display = color.canvas_display(&doc).unwrap();
        let px = [[0.25, 0.25, 0.25, 1.0]];
        let preview = CameraRawDialog::image(&px, 1, 1, &display);
        let expected = display.to_rgba8(&photocraft_compose::Buffer::filled(Rect::new(0, 0, 1, 1), px[0]));
        assert_eq!(preview.pixels[0].to_array(), expected.pixels[..4]);
        assert_ne!(preview.pixels[0].r(), 64, "linear light must be encoded before display");
    }

    #[test]
    fn histogram_follows_preview_and_view_changes_do_not_render_or_edit() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        let initial = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        assert_eq!(initial["histogram"]["samples"], 64 * 48);
        assert_eq!(initial["histogram"]["red"][128], 64 * 48);
        let updated = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
        assert_ne!(updated["histogram"]["red"], initial["histogram"]["red"]);
        let before = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": true}})).unwrap().unwrap();
        assert_eq!(before["histogram"]["source"], "before");
        assert_eq!(before["histogram"]["red"], initial["histogram"]["red"]);
        assert_eq!(before["previewRevision"], updated["previewRevision"]);
        let after = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": false, "set": {"exposure": 1}}})).unwrap().unwrap();
        assert_eq!(after["histogram"], updated["histogram"]);
        assert_eq!(after["previewRevision"], updated["previewRevision"]);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        let pixel = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(0, 0);
        assert!((pixel[0] - 128.0 / 255.0).abs() < 0.0001);
    }

    /// #4100 in the Camera Raw 16.1.1 layout: each edit is an undo step, `undo`/`redo` walk them.
    #[test]
    fn undo_and_redo_walk_the_edits() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 20}}})).unwrap().unwrap();
        let p = |app: &PhotocraftApp| {
            let d = app.camera_raw.as_ref().unwrap();
            (d.params.exposure, d.params.contrast)
        };
        assert_eq!(p(&app), (1.0, 20.0));
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"undo": true}})).unwrap().unwrap();
        assert_eq!(p(&app), (1.0, 0.0));
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"undo": true}})).unwrap().unwrap();
        assert_eq!(p(&app), (0.0, 0.0));
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"redo": true}})).unwrap().unwrap();
        assert_eq!(p(&app), (1.0, 0.0));
        // Layout and panel changes are not edits.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"beforeAfter": "leftRight"}})).unwrap().unwrap();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"undo": true}})).unwrap().unwrap();
        assert_eq!(p(&app), (0.0, 0.0));
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    }

    /// Closing Camera Raw's window is Cancel. Without a multi-window backend egui embeds the
    /// "window" (`ViewportClass::EmbeddedWindow`), and the close request comes from the window
    /// it is drawn in; the overlay fallback (`embed_viewports`) keeps the dialog open.
    #[test]
    fn closing_the_camera_raw_window_cancels() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        let frame = |app: &mut PhotocraftApp, close: bool| {
            let mut input = egui::RawInput::default();
            if close {
                let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
                info.events.push(egui::ViewportEvent::Close);
            }
            ctx.run_ui(input, |ui| show(app, ui.ctx())).textures_delta.clear();
        };
        open(&mut app, &ctx).unwrap();
        frame(&mut app, false);
        assert!(app.camera_raw.is_some(), "drawn over the main window");
        ctx.set_embed_viewports(false);
        frame(&mut app, false);
        assert!(app.camera_raw.is_some(), "drawn in its window");
        frame(&mut app, true);
        assert!(app.camera_raw.is_none(), "closing the window is Cancel");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    }

    #[test]
    fn smart_filter_double_click_edits_the_stored_camera_raw_filter_in_its_own_dialog() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        // An older editor could save stacked curve inputs; the filter must stay editable.
        let legacy = json!([[0.0, 0.0], [60.0, 40.0], [60.0, 200.0], [255.0, 255.0]]);
        app.run("filter.cameraRaw", json!({"exposure": 1.0, "contrast": 20})).unwrap();
        app.run("layer.smartFilter.setParams", json!({"index": 0, "params": {"pointCurve": legacy}})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let smart = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
            _ => panic!("not a smart object"),
        };
        let steps = app.session.active().unwrap().history.past_len();

        let sm = smart(&app);
        crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
        assert!(app.ui.dialogs.is_empty(), "no generic parameter form");
        let opened = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {}})).unwrap().unwrap();
        assert_eq!(opened["smartFilter"], 0);
        assert_eq!(opened["params"]["exposure"], 1.0);
        assert_eq!(opened["params"]["pointCurve"], legacy);
        // The preview starts from the pixels below the filter, not the filtered cache.
        let before = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": true}})).unwrap().unwrap();
        assert_eq!(before["histogram"]["red"][128], 64 * 48);

        // Resetting a control to its default must overwrite the stored value.
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.0}, "before": false, "commit": true}})).unwrap();
        assert!(r.is_ok(), "{r:?}");
        assert!(app.camera_raw.is_none());
        let sm = smart(&app);
        assert_eq!(sm.smart_filters.len(), 1, "edited in place, not added");
        assert_eq!(sm.smart_filters[0].params["exposure"], 0.0);
        assert_eq!(sm.smart_filters[0].params["contrast"], 20.0);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(smart(&app).smart_filters[0].params["exposure"], 1.0);

        // A broken new curve is still rejected; an unknown index is an error, not a panic.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 0}})).unwrap().unwrap();
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"pointCurve": [[10, 10]]}}})).unwrap().is_err());
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 7}})).unwrap().is_err());
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": "x"}})).unwrap().is_err());
    }

    #[test]
    fn distant_sparse_pixels_cannot_trigger_an_unbounded_proxy_scan() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let mut source = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
        source.write_region(Rect::new(30_000, 30_000, 30_001, 30_001), &[0.5, 0.5, 0.5, 1.0]);
        let result = open_pixels(&mut app, &ctx, layer, source, CameraRaw::default(), Coverage::None, Target::NewFilter);
        assert!(result.unwrap_err().contains("bounds are too large"));
        assert!(app.camera_raw.is_none());
    }

    #[test]
    fn dialog_drives_the_engine() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        assert_eq!(r["proxy"], json!([64, 48]));
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "vignetteAmount": -40}}})).unwrap().unwrap();
        assert_eq!(r["params"]["exposure"], 1.0);
        assert_eq!(app.camera_raw.as_ref().unwrap().command_params(), json!({"exposure": 1.0, "vignetteAmount": -40.0}));
        let before = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert!(app.camera_raw.is_none());
        let after = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
        assert!(after[0] > before[0] + 0.1, "{before:?} → {after:?}");
        // Cancel leaves the document alone.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert_eq!(r["cancelled"], true);
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": "x"}}})).unwrap().is_err());
    }

    /// An app whose importer is the real one, opening a synthetic DNG.
    fn raw_app() -> (PhotocraftApp, Vec<u8>) {
        use photocraft_raw::testgen::{DngSpec, mosaic, scene};
        let services = crate::Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let (w, h) = (48, 32);
        let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
        spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
        (PhotocraftApp::new(photocraft_engine::Session::new(), services), spec.build())
    }

    fn mean_red(app: &PhotocraftApp) -> f32 {
        let st = app.session.active().unwrap();
        let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        (0..8).map(|i| s.rgba(4 + i * 4, 16)[0]).sum::<f32>() / 8.0
    }

    #[test]
    fn raw_opens_in_camera_raw_and_open_redevelops() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        let warnings = app.open_bytes("IMG_0001.dng", &dng).unwrap();
        assert!(warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)), "{warnings:?}");
        // A raw has no embedded profile (ProPhoto RGB is the space it is developed into), so Ask
        // When Opening (on by default) has nothing to ask about.
        assert!(app.ui.dialogs.is_empty(), "no profile prompt for a raw");
        let raw = app.pending_raw_open.take().expect("a raw open queues the dialog");
        open_raw(&mut app, &ctx, raw).unwrap();
        let d = app.camera_raw.as_ref().unwrap();
        assert_eq!(d.target, Target::OpenRaw);
        assert_eq!(d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)["openingRaw"]["name"], "IMG_0001.dng");
        let before = mean_red(&app);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "temperature": 30, "contrast": 20}}})).unwrap().unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["redeveloped"], true, "{r}");
        assert!(r.get("filter").is_some(), "contrast is applied as a Camera Raw step: {r}");
        assert!(app.camera_raw.is_none());
        assert_eq!(app.session.documents().len(), 1);
        let st = app.session.active().unwrap();
        assert_eq!(st.saved_revision, st.revision, "Open leaves the document unmodified");
        assert_eq!(st.doc.depth, photocraft_color::SampleType::U16);
        assert!(mean_red(&app) > before + 0.05, "{before} → {}", mean_red(&app));
    }

    fn pixels(app: &PhotocraftApp) -> Vec<f32> {
        let st = app.session.active().unwrap();
        let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        (0..32).flat_map(|y| (0..48).map(move |x| (x, y))).flat_map(|(x, y)| s.rgba(x, y)).collect()
    }

    #[test]
    fn the_demosaic_method_is_chosen_at_open() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.open_bytes("IMG_0002.dng", &dng).unwrap();
        let ahd = pixels(&app);
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        let d = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"demosaic": "bilinear"}})).unwrap().unwrap();
        assert_eq!(d["openingRaw"]["demosaic"], "bilinear");
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"demosaic": "amaze"}})).unwrap().is_err());
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["redeveloped"], true, "another method re-develops: {r}");
        assert!(r.get("filter").is_none(), "nothing else to apply: {r}");
        let bilinear = pixels(&app);
        assert!(ahd.iter().zip(&bilinear).any(|(a, b)| (a - b).abs() > 1e-3), "bilinear differs from AHD");
        let st = app.session.active().unwrap();
        assert_eq!(st.saved_revision, st.revision);

        // The preference sets the method of the open itself; Open with it changes nothing.
        app.run("prefs.set", json!({"path": "rawDefaults.demosaic", "value": "bilinear"})).unwrap();
        app.open_bytes("IMG_0003.dng", &dng).unwrap();
        assert_eq!(pixels(&app), bilinear, "developed with the preference's method");
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        let d = app.camera_raw.as_ref().unwrap().describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview);
        assert_eq!(d["openingRaw"]["demosaic"], "bilinear");
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["redeveloped"], false, "{r}");

        // Only the open-time dialog has the choice.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"demosaic": "ahd"}})).unwrap().is_err());
    }

    #[test]
    fn shadows_of_a_raw_open_act_on_the_raw_data_and_the_preview_shows_it() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.open_bytes("IMG_0002.dng", &dng).unwrap();
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        assert!(app.camera_raw.as_ref().unwrap().raw_curve.is_some(), "the preview knows how the raw was developed");
        let mean = |px: &[[f32; 4]]| px.iter().map(|p| p[0] + p[1] + p[2]).sum::<f32>() / px.len() as f32;
        let preview_before = mean(&app.camera_raw.as_ref().unwrap().processed);
        let before = mean_red(&app);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"shadows": 100}}})).unwrap().unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        d.render(&ctx);
        assert!(mean(&d.processed) > preview_before + 0.01, "{preview_before} → {}", mean(&d.processed));
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["redeveloped"], true, "{r}");
        assert!(r.get("filter").is_none(), "Shadows acts while developing, not as a later filter step: {r}");
        assert!(mean_red(&app) > before + 0.01, "{before} → {}", mean_red(&app));
    }

    #[test]
    fn clarity_texture_and_dehaze_of_a_raw_open_act_on_the_raw_data() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.open_bytes("IMG_0003.dng", &dng).unwrap();
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        let before = mean_red(&app);
        let preview_before = app.camera_raw.as_ref().unwrap().processed.clone();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"clarity": 60, "texture": 40, "dehaze": 50}}})).unwrap().unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        d.render(&ctx);
        assert_ne!(d.processed, preview_before, "the preview shows the raw-stage change");
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["redeveloped"], true, "{r}");
        assert!(r.get("filter").is_none(), "they act while developing, not as a later filter step: {r}");
        assert!(mean_red(&app) < before, "Dehaze darkens: {before} → {}", mean_red(&app));
    }

    #[test]
    fn background_redevelop_keeps_the_tab_position_and_view() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 10, "height": 10})).unwrap();
        app.open_bytes("mid.dng", &dng).unwrap();
        app.run("file.new", json!({"width": 12, "height": 12})).unwrap();
        app.sync_views();
        let raw = app.pending_raw_open.take().unwrap();
        let old_id = raw.document;
        assert_eq!(document_index(&app, old_id), Some(1));
        app.ui.views[1].zoom = 3.0;
        open_raw(&mut app, &ctx, raw).unwrap();
        let before = mean_red(&app);
        // The re-develop and the Camera Raw step run as jobs; the UI thread only polls.
        app.background_jobs = true;
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "contrast": 20}}})).unwrap().unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["pending"], true, "{r}");
        assert!(app.camera_raw.is_none());
        for _ in 0..3000 {
            crate::jobs_ui::tick(&mut app, &ctx);
            if app.raw_redevelop.is_none() && !app.session.has_jobs() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.raw_redevelop.is_none() && !app.session.has_jobs(), "the jobs never finished");
        let docs = app.session.documents();
        assert_eq!(docs.len(), 3);
        assert_eq!((docs[0].doc.size.width, docs[2].doc.size.width), (10, 12), "the other tabs keep their places");
        assert_ne!(docs[1].doc.id, old_id, "re-developed");
        assert_eq!(docs[1].doc.depth, photocraft_color::SampleType::U16);
        assert_eq!(docs[1].saved_revision, docs[1].revision, "Open leaves the document unmodified");
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.ui.views[1].zoom, 3.0, "the view carries over to the re-developed document");
        assert!(mean_red(&app) > before + 0.05, "{before} → {}", mean_red(&app));
    }

    #[test]
    fn cancel_closes_the_raw_and_the_preference_skips_the_dialog() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.open_bytes("a.dng", &dng).unwrap();
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert_eq!(r["cancelled"], true);
        assert!(app.session.documents().is_empty(), "Cancel closes the raw being opened");
        // Unchanged settings: Open keeps the default develop and adds no step.
        app.open_bytes("b.dng", &dng).unwrap();
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r, json!({"document": 0, "redeveloped": false}));
        // Preference off: raws open straight away.
        app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": false})).unwrap();
        app.open_bytes("c.dng", &dng).unwrap();
        assert!(app.pending_raw_open.is_none());
        // A non-raw never queues the dialog; a closed document is an error, not a panic.
        assert!(
            open_raw(
                &mut app,
                &ctx,
                RawOpen {
                    document: photocraft_doc::DocId(u64::MAX),
                    name: "x".into(),
                    path: None,
                    bytes: None,
                    developed_with: Default::default(),
                    demosaic: Default::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn save_and_load_settings_round_trip_through_xmp_files() {
        use crate::file_dialog::FileDialogAnswer;
        use std::sync::{Arc, Mutex};
        type Written = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
        let written: Written = Arc::default();
        let w = written.clone();
        let foreign = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Clarity2012="+25" crs:RedHue="-5"/>
            </rdf:RDF></x:xmpmeta>"#;
        let answers = vec![
            Some(FileDialogAnswer::SaveTo("/out/Look.xmp".into())),
            Some(FileDialogAnswer::Contents("Look.xmp".into(), Vec::new())),
            Some(FileDialogAnswer::Contents("Other.xmp".into(), foreign.to_vec())),
            Some(FileDialogAnswer::Contents("Broken.xmp".into(), b"<nope".to_vec())),
        ];
        let services = crate::Services {
            file_dialog: Some(crate::file_dialog::fake(answers).0),
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.lock().unwrap().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let frame = |app: &mut PhotocraftApp| app.poll_file_dialog(&egui::Context::default(), None);
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.75, "contrast": -20, "hslSat": [0, 0, 0, 30, 0, 0, 0, 0]}}}))
            .unwrap()
            .unwrap();
        let saved_params = app.camera_raw.as_ref().unwrap().params.clone();
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"saveSettings": true, "before": true}})).unwrap().is_err());
        assert_eq!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"saveSettings": true}})).unwrap().unwrap()["fileDialog"], "save");
        frame(&mut app);
        let (path, bytes) = written.lock().unwrap()[0].clone();
        assert_eq!(path, "/out/Look.xmp");
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("crs:Exposure2012=\"+0.75\"") && text.contains("crs:SaturationAdjustmentGreen=\"+30\""), "{text}");
        // Reset, then load the saved file back (the fake dialog's second answer, with its bytes).
        app.camera_raw.as_mut().unwrap().params = CameraRaw::default();
        let mut svc = crate::file_dialog::fake(vec![Some(FileDialogAnswer::Contents("Look.xmp".into(), bytes))]).0;
        std::mem::swap(app.services.file_dialog.as_mut().unwrap(), &mut svc);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
        frame(&mut app);
        assert_eq!(app.camera_raw.as_ref().unwrap().params, saved_params);
        std::mem::swap(app.services.file_dialog.as_mut().unwrap(), &mut svc);
        // The original service's next answer is the empty file: an error, settings untouched.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
        frame(&mut app);
        assert_eq!(app.camera_raw.as_ref().unwrap().params, saved_params);
        // A foreign preset applies what it holds over the current settings and reports the rest.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
        frame(&mut app);
        let p = &app.camera_raw.as_ref().unwrap().params;
        assert_eq!((p.clarity, p.exposure), (25.0, 0.75));
        assert!(app.ui.status.contains("Other.xmp") && app.ui.status.contains('1'), "{}", app.ui.status);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
        frame(&mut app);
        assert_eq!(app.camera_raw.as_ref().unwrap().params.clarity, 25.0);
    }
}
