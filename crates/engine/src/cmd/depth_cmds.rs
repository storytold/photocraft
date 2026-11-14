//! Monocular depth estimation: the model store (download, import, status) and `image.estimateDepth`.
//!
//! The weights (about 95 MB) are not shipped. The desktop and the CLI inject a models folder and
//! a [`FetchFn`] (a background download); the engine owns the state, so the GUI, CLI and MCP see
//! the same status. Nothing is downloaded until `depth.downloadModel` runs: running it is the
//! consent. The web build injects nothing, so these commands are disabled there.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

use photocraft_depth::{DepthError, DepthMap, MAX_PIXELS, Model, WEIGHTS, WeightFile};
use photocraft_doc::{AlphaChannel, Document};
use photocraft_geom::Rect;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::channel_cmds::{channel_format, plane_surface};
use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session};

/// What a running download reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchEvent {
    /// Bytes received so far over the bytes of every weight file.
    Progress { done: u64, total: u64 },
    /// The download ended (`Err` with a message, also when it was cancelled).
    Done(std::result::Result<(), String>),
}

/// Starts downloading every file of [`WEIGHTS`] into the given folder (verifying size and sha256)
/// on a background thread and returns at once. The flag asks it to stop. Only public model files
/// are requested; no document or personal data is sent.
pub type FetchFn = Box<dyn FnMut(PathBuf, Arc<AtomicBool>) -> std::result::Result<Receiver<FetchEvent>, String> + Send>;

/// A depth estimator over interleaved RGB (0..1): the test hook that replaces the model.
pub type Estimator = Arc<dyn Fn(&[f32], u32, u32) -> std::result::Result<DepthMap, String> + Send + Sync>;

type EstimateFn = dyn Fn(&[f32], u32, u32, &dyn Fn(f32) -> bool) -> Result<DepthMap> + Send + Sync;

/// Pins the closure's signature so its reference arguments are inferred for any lifetime.
fn boxed<F: Fn(&[f32], u32, u32, &dyn Fn(f32) -> bool) -> Result<DepthMap> + Send + Sync + 'static>(f: F) -> Arc<EstimateFn> {
    Arc::new(f)
}

/// Name of the file that remembers which downloads passed their sha256 check.
const MARKER: &str = "verified.txt";
/// Longest side handed to the network (it resizes to 518 anyway).
const MODEL_INPUT_SIDE: usize = 1024;
/// Longest side of the grid the depth is refined on, before it is stretched to the document.
const REFINE_SIDE: usize = 4096;
const MAX_NAME: usize = 255;
const NOT_INSTALLED: &str = "depth model not installed: run depth.downloadModel";

#[derive(Default)]
pub struct DepthState {
    /// The folder the weights live in; `None` where depth is unavailable (the web build).
    pub models_dir: Option<PathBuf>,
    pub fetch: Option<FetchFn>,
    /// Replaces the model when set (tests).
    pub estimator: Option<Estimator>,
    progress: Option<(u64, u64)>,
    error: Option<String>,
    pending: Option<Receiver<FetchEvent>>,
    stop: Option<Arc<AtomicBool>>,
    cancelled: bool,
}

impl DepthState {
    /// Apply download events that arrived since the last call; true when something changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else { return false };
        let mut changed = false;
        let mut finished: Option<std::result::Result<(), String>> = None;
        loop {
            match rx.try_recv() {
                Ok(FetchEvent::Progress { done, total }) => {
                    self.progress = Some((done, total));
                    changed = true;
                }
                Ok(FetchEvent::Done(r)) => {
                    finished = Some(r);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = Some(Err("The download stopped unexpectedly".into()));
                    break;
                }
            }
        }
        let Some(result) = finished else { return changed };
        self.pending = None;
        self.stop = None;
        self.progress = None;
        self.error = match result {
            Ok(()) => self.models_dir.as_deref().and_then(|d| write_marker(d).err()),
            Err(_) if self.cancelled => None,
            Err(e) => Some(e),
        };
        self.cancelled = false;
        true
    }

    pub fn downloading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn installed(&self) -> bool {
        self.models_dir.as_deref().is_some_and(installed)
    }

    fn supported(&self) -> bool {
        self.models_dir.is_some() && !cfg!(target_arch = "wasm32")
    }
}

/// The marker lines for every weight: `<sha256> <size> <name>`.
fn marker_text() -> String {
    WEIGHTS.iter().map(|w| format!("{} {} {}\n", w.sha256, w.size, w.name)).collect()
}

fn write_marker(dir: &Path) -> std::result::Result<(), String> {
    std::fs::write(dir.join(MARKER), marker_text()).map_err(|e| format!("could not record the verified model files: {e}"))
}

/// Both files are present with their exact sizes and were verified (by a download or an import).
/// The hash is not recomputed here: this runs on every status poll.
fn installed(dir: &Path) -> bool {
    let marker = std::fs::read_to_string(dir.join(MARKER)).unwrap_or_default();
    marker == marker_text() && WEIGHTS.iter().all(|w| std::fs::metadata(dir.join(w.name)).is_ok_and(|m| m.is_file() && m.len() == w.size))
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

fn empty_params(p: &Value) -> Result<()> {
    match p {
        Value::Null => Ok(()),
        Value::Object(o) if o.is_empty() => Ok(()),
        _ => Err(bad("this command takes an empty object")),
    }
}

fn status_json(s: &DepthState) -> Value {
    json!({
        "supported": s.supported(),
        "installed": s.installed(),
        "downloading": s.progress.filter(|_| s.downloading()).or(s.downloading().then_some((0, 0))).map(|(done, total)| json!({"done": done, "total": total})),
        "error": s.error,
    })
}

fn model_status(s: &mut Session, p: &Value) -> Result<Value> {
    empty_params(p)?;
    s.depth.poll();
    Ok(status_json(&s.depth))
}

fn enabled_download(s: &Session) -> std::result::Result<(), String> {
    if s.depth.fetch.is_none() || s.depth.models_dir.is_none() {
        return Err("Depth model downloads are unavailable in this frontend".into());
    }
    if s.depth.downloading() {
        return Err("The depth model is already downloading".into());
    }
    if s.depth.installed() {
        return Err("The depth model is already installed".into());
    }
    Ok(())
}

fn download_model(s: &mut Session, p: &Value) -> Result<Value> {
    empty_params(p)?;
    s.depth.poll();
    enabled_download(s).map_err(bad)?;
    let dir = s.depth.models_dir.clone().ok_or_else(|| bad("no models folder"))?;
    std::fs::create_dir_all(&dir).map_err(|e| bad(format!("could not create the models folder: {e}")))?;
    // A stale marker must not outlive files that are about to be replaced.
    let _ = std::fs::remove_file(dir.join(MARKER));
    let stop = Arc::new(AtomicBool::new(false));
    let start = s.depth.fetch.as_mut().ok_or_else(|| bad("Depth model downloads are unavailable in this frontend"))?;
    match start(dir, stop.clone()) {
        Ok(rx) => {
            s.depth.pending = Some(rx);
            s.depth.stop = Some(stop);
            s.depth.error = None;
            s.depth.cancelled = false;
            s.depth.progress = Some((0, WEIGHTS.iter().map(|w| w.size).sum()));
        }
        Err(message) => {
            s.depth.error = Some(message.clone());
            return Err(bad(message));
        }
    }
    Ok(status_json(&s.depth))
}

fn enabled_cancel(s: &Session) -> std::result::Result<(), String> {
    if s.depth.downloading() { Ok(()) } else { Err("No depth model download is running".into()) }
}

fn cancel_download(s: &mut Session, p: &Value) -> Result<Value> {
    empty_params(p)?;
    enabled_cancel(s).map_err(bad)?;
    if let Some(stop) = &s.depth.stop {
        stop.store(true, Ordering::SeqCst);
    }
    s.depth.cancelled = true;
    Ok(status_json(&s.depth))
}

fn enabled_import(s: &Session) -> std::result::Result<(), String> {
    if s.depth.models_dir.is_none() {
        return Err("The depth model store is unavailable in this frontend".into());
    }
    if s.depth.downloading() {
        return Err("The depth model is downloading".into());
    }
    Ok(())
}

fn path_param(p: &Value, key: &str) -> Result<PathBuf> {
    let v = p.get(key).and_then(Value::as_str).ok_or_else(|| bad(format!("`{key}` must be a file path")))?;
    if v.is_empty() || v.len() > 4096 || v.contains('\0') {
        return Err(bad(format!("`{key}` must be a file path")));
    }
    Ok(PathBuf::from(v))
}

/// Copy `src` to `dest`, which must be exactly `weight.size` bytes with the pinned sha256.
fn import_file(src: &Path, dest: &Path, weight: &WeightFile) -> Result<()> {
    let meta = std::fs::metadata(src).map_err(|e| bad(format!("could not read {}: {e}", src.display())))?;
    if !meta.is_file() {
        return Err(bad(format!("{} is not a file", src.display())));
    }
    if meta.len() != weight.size {
        return Err(bad(format!("{} is {} bytes; {} must be {} bytes", src.display(), meta.len(), weight.name, weight.size)));
    }
    let file = std::fs::File::open(src).map_err(|e| bad(format!("could not read {}: {e}", src.display())))?;
    // Reading is bounded by the pinned size even if the file grows while it is read.
    let mut input = file.take(weight.size + 1);
    let _ = std::fs::remove_file(dest);
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(dest).map_err(|e| bad(format!("could not write {}: {e}", dest.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = input.read(&mut buf).map_err(|e| bad(format!("could not read {}: {e}", src.display())))?;
        if n == 0 {
            break;
        }
        let chunk = buf.get(..n).ok_or_else(|| bad("read overran its buffer"))?;
        hasher.update(chunk);
        out.write_all(chunk).map_err(|e| bad(format!("could not write {}: {e}", dest.display())))?;
        total += n as u64;
    }
    out.flush().map_err(|e| bad(format!("could not write {}: {e}", dest.display())))?;
    let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if total != weight.size || !hex.eq_ignore_ascii_case(weight.sha256) {
        return Err(bad(format!("{} is not the expected {} (checksum mismatch)", src.display(), weight.name)));
    }
    Ok(())
}

fn load_model_files(s: &mut Session, p: &Value) -> Result<Value> {
    s.depth.poll();
    enabled_import(s).map_err(bad)?;
    if !p.is_object() {
        return Err(bad("expected {\"dinov2\": path, \"head\": path}"));
    }
    let sources = [path_param(p, "dinov2")?, path_param(p, "head")?];
    let dir = s.depth.models_dir.clone().ok_or_else(|| bad("no models folder"))?;
    std::fs::create_dir_all(&dir).map_err(|e| bad(format!("could not create the models folder: {e}")))?;
    let staged: Vec<PathBuf> = WEIGHTS.iter().map(|w| dir.join(format!("{}.import", w.name))).collect();
    let result = (|| -> Result<()> {
        for ((src, dest), w) in sources.iter().zip(&staged).zip(&WEIGHTS) {
            import_file(src, dest, w)?;
        }
        let _ = std::fs::remove_file(dir.join(MARKER));
        for (dest, w) in staged.iter().zip(&WEIGHTS) {
            std::fs::rename(dest, dir.join(w.name)).map_err(|e| bad(format!("could not install {}: {e}", w.name)))?;
        }
        write_marker(&dir).map_err(bad)
    })();
    for f in &staged {
        let _ = std::fs::remove_file(f);
    }
    result?;
    s.depth.error = None;
    Ok(status_json(&s.depth))
}

// ---------------------------------------------------------------------------------------------
// image.estimateDepth

/// Averages source pixels into a smaller grid (box filter), fed one band of rows at a time.
struct Accum {
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
    ch: usize,
    sums: Vec<f32>,
    counts: Vec<u32>,
}

impl Accum {
    /// A grid for `sw` x `sh` sources whose longer side is at most `max_side`.
    fn new(sw: usize, sh: usize, ch: usize, max_side: usize) -> Self {
        let (dw, dh) = fit(sw, sh, max_side);
        Self { sw, sh, dw, dh, ch, sums: vec![0.0; dw * dh * ch], counts: vec![0; dw * dh] }
    }

    /// Add source row `y` (`sw * ch` values).
    fn add_row(&mut self, y: usize, row: &[f32]) {
        if y >= self.sh || row.len() < self.sw * self.ch {
            return;
        }
        let dy = (y * self.dh / self.sh).min(self.dh - 1);
        for x in 0..self.sw {
            let dx = (x * self.dw / self.sw).min(self.dw - 1);
            let i = dy * self.dw + dx;
            self.counts[i] += 1;
            for c in 0..self.ch {
                self.sums[i * self.ch + c] += row[x * self.ch + c];
            }
        }
    }

    fn finish(mut self) -> Vec<f32> {
        for (i, n) in self.counts.iter().enumerate() {
            let inv = 1.0 / (*n).max(1) as f32;
            for c in 0..self.ch {
                self.sums[i * self.ch + c] *= inv;
            }
        }
        self.sums
    }
}

/// `w` x `h` scaled down (never up) so the longer side is at most `max_side`; at least 1 x 1.
fn fit(w: usize, h: usize, max_side: usize) -> (usize, usize) {
    let long = w.max(h);
    if long <= max_side {
        return (w.max(1), h.max(1));
    }
    let scale = max_side as f64 / long as f64;
    (((w as f64 * scale).round() as usize).max(1), ((h as f64 * scale).round() as usize).max(1))
}

/// Bilinear resize of one plane (pixel centres aligned).
fn resize_plane(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    if (sw, sh) == (dw, dh) {
        return src.to_vec();
    }
    if sw == 0 || sh == 0 || src.len() < sw * sh {
        return vec![0.0; dw * dh];
    }
    let (fx, fy) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    let xs: Vec<(usize, usize, f32)> = (0..dw)
        .map(|x| {
            let u = ((x as f32 + 0.5) * fx - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = u.floor() as usize;
            (x0, (x0 + 1).min(sw - 1), u - x0 as f32)
        })
        .collect();
    let mut out = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        let v = ((y as f32 + 0.5) * fy - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = v.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let wy = v - y0 as f32;
        let (r0, r1) = (&src[y0 * sw..(y0 + 1) * sw], &src[y1 * sw..(y1 + 1) * sw]);
        out.extend(xs.iter().map(|&(x0, x1, wx)| {
            let a = r0[x0] + (r0[x1] - r0[x0]) * wx;
            let b = r1[x0] + (r1[x1] - r1[x0]) * wx;
            a + (b - a) * wy
        }));
    }
    out
}

/// Load the network from `dir`. The candle model is not `Send`, so it is built on the job's own
/// thread for each estimate instead of being cached in the session.
fn load_model(dir: &Path) -> Result<Model> {
    let read = |w: &WeightFile| -> Result<Vec<u8>> {
        let path = dir.join(w.name);
        let file = std::fs::File::open(&path).map_err(|e| bad(format!("could not read {}: {e}", path.display())))?;
        let mut bytes = Vec::with_capacity(usize::try_from(w.size).unwrap_or(0));
        file.take(w.size + 1).read_to_end(&mut bytes).map_err(|e| bad(format!("could not read {}: {e}", path.display())))?;
        if bytes.len() as u64 != w.size {
            return Err(bad(format!("{} has the wrong size; load it again with depth.loadModelFiles", w.name)));
        }
        Ok(bytes)
    };
    let (dino, head) = (read(&WEIGHTS[0])?, read(&WEIGHTS[1])?);
    Model::load(&dino, &head).map_err(depth_error)
}

fn depth_error(e: DepthError) -> EngineError {
    match e {
        DepthError::Cancelled => EngineError::Cancelled,
        other => EngineError::Other(other.to_string()),
    }
}

fn channel_name(p: &Value) -> Result<String> {
    let obj = match p {
        Value::Null => None,
        Value::Object(o) => Some(o),
        _ => return Err(bad("expected {\"channel\": name}")),
    };
    let name = match obj.and_then(|o| o.get("channel")) {
        None => "Depth".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(bad("`channel` must be a channel name")),
    };
    if name.trim().is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(bad(format!("`channel` must be a name of 1 to {MAX_NAME} characters")));
    }
    Ok(name)
}

fn enabled_estimate(s: &Session) -> std::result::Result<(), String> {
    s.active().ok_or("no document open")?;
    Ok(())
}

/// Composite the visible image and reduce it: RGB for the network and luma for the guide.
fn read_image(doc: &Document, ctx: &crate::jobs::JobCtx) -> Result<(Vec<f32>, (usize, usize), Vec<f32>, (usize, usize))> {
    let rect = doc.bounds();
    let (w, h) = (rect.width().max(0) as usize, rect.height().max(0) as usize);
    if w == 0 || h == 0 {
        return Err(bad("the document is empty"));
    }
    if (w as u64).saturating_mul(h as u64) > MAX_PIXELS / 2 {
        return Err(bad("the image is too large for depth estimation"));
    }
    let fmt = photocraft_color::PixelFormat::RGBA8;
    let flat = ctx
        .stage(0.0, 0.15, "Reading image", |ctl| photocraft_compose::composite_to_surface_with_interrupt(doc, rect, fmt, Some([1.0; 3]), ctl))
        .map_err(|_| EngineError::Cancelled)?;
    let mut rgb = Accum::new(w, h, 3, MODEL_INPUT_SIDE);
    let mut luma = Accum::new(w, h, 1, REFINE_SIDE);
    let mut row_rgb = vec![0.0f32; w * 3];
    let mut row_luma = vec![0.0f32; w];
    let band = 128i32;
    let mut y = rect.y0;
    while y < rect.y1 {
        if ctx.cancelled() {
            return Err(EngineError::Cancelled);
        }
        let end = y.saturating_add(band).min(rect.y1);
        let px = flat.read_region(Rect::new(rect.x0, y, rect.x1, end));
        for (r, line) in px.chunks_exact(w * 4).enumerate() {
            for (x, p) in line.chunks_exact(4).enumerate() {
                let (rr, gg, bb) = (p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0), p[2].clamp(0.0, 1.0));
                row_rgb[x * 3..x * 3 + 3].copy_from_slice(&[rr, gg, bb]);
                row_luma[x] = 0.299 * rr + 0.587 * gg + 0.114 * bb;
            }
            let sy = (y - rect.y0) as usize + r;
            rgb.add_row(sy, &row_rgb);
            luma.add_row(sy, &row_luma);
        }
        y = end;
    }
    let (rd, ld) = ((rgb.dw, rgb.dh), (luma.dw, luma.dh));
    Ok((rgb.finish(), rd, luma.finish(), ld))
}

/// Run the estimator and refine its map to the document size: edges follow the photo.
fn estimate_plane(doc: &Document, ctx: &crate::jobs::JobCtx, estimator: &EstimateFn) -> Result<Vec<f32>> {
    let rect = doc.bounds();
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let (rgb, (rw, rh), luma, (lw, lh)) = read_image(doc, ctx)?;
    ctx.progress(0.15, "Estimating depth");
    let map = estimator(&rgb, rw as u32, rh as u32, &|p| {
        ctx.progress(0.15 + 0.65 * p, "");
        ctx.cancelled()
    })?;
    drop(rgb);
    if ctx.cancelled() {
        return Err(EngineError::Cancelled);
    }
    let (mw, mh) = (map.width as usize, map.height as usize);
    if mw == 0 || mh == 0 || map.data.len() != mw.saturating_mul(mh) || map.data.iter().any(|v| !v.is_finite()) {
        return Err(bad("the depth model returned an invalid map"));
    }
    ctx.progress(0.8, "Refining");
    let coarse = resize_plane(&map.data, mw, mh, lw, lh);
    let radius = (lw.max(lh) / 200).clamp(2, 24);
    let mut refined = photocraft_algo::matting::guided_filter_gray(&luma, &coarse, lw, lh, radius, 1e-3);
    drop((luma, coarse));
    for v in &mut refined {
        *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    }
    if ctx.cancelled() {
        return Err(EngineError::Cancelled);
    }
    ctx.progress(0.95, "");
    Ok(resize_plane(&refined, lw, lh, w, h))
}

fn estimate_depth(s: &mut Session, p: &Value) -> Result<Value> {
    let name = channel_name(p)?;
    s.active().ok_or(EngineError::NoDocument)?;
    s.depth.poll();
    let estimator = match s.depth.estimator.clone() {
        Some(e) => boxed(move |rgb, w, h, _| e(rgb, w, h).map_err(EngineError::Other)),
        None => {
            let dir = s.depth.models_dir.clone().filter(|d| installed(d)).ok_or_else(|| bad(NOT_INSTALLED))?;
            boxed(move |rgb, w, h, interrupt| {
                let model = load_model(&dir)?;
                model.estimate(rgb, w, h, interrupt).map_err(depth_error)
            })
        }
    };
    let started = web_time::Instant::now();
    crate::jobs::edit_job(
        s,
        "Estimate Depth",
        move |doc, _, ctx| {
            let plane = estimate_plane(doc, ctx, &*estimator)?;
            let rect = doc.bounds();
            let surface = plane_surface(&plane, rect, channel_format(doc));
            let index = match doc.channels.iter().position(|c| c.name == name) {
                Some(i) => {
                    doc.channels[i].surface = surface;
                    i
                }
                None => {
                    doc.channels.push(AlphaChannel::new(name, surface));
                    doc.channels.len() - 1
                }
            };
            Ok((index, rect.width(), rect.height()))
        },
        move |(index, w, h)| json!({"channel": index, "width": w, "height": h, "ms": started.elapsed().as_millis() as u64}),
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "depth.modelStatus",
            label: "Depth Model Status",
            menu: &[],
            shortcut: None,
            params: r#"{} → {supported,installed,downloading:{done,total}|null,error:string|null}"#,
            enabled: always,
            run: model_status,
            journal: false,
        },
        CommandSpec {
            id: "depth.downloadModel",
            label: "Download Depth Model",
            menu: &[],
            shortcut: None,
            params: r#"{} → status: downloads the depth model (about 95 MB) from its pinned public source; running this is the consent"#,
            enabled: enabled_download,
            run: download_model,
            journal: false,
        },
        CommandSpec {
            id: "depth.cancelDownload",
            label: "Cancel Depth Model Download",
            menu: &[],
            shortcut: None,
            params: r#"{} → status"#,
            enabled: enabled_cancel,
            run: cancel_download,
            journal: false,
        },
        CommandSpec {
            id: "depth.loadModelFiles",
            label: "Load Depth Model Files",
            menu: &[],
            shortcut: None,
            params: r#"{"dinov2":path,"head":path} → status: installs the two downloaded model files after checking their size and sha256"#,
            enabled: enabled_import,
            run: load_model_files,
            journal: false,
        },
        CommandSpec {
            id: "image.estimateDepth",
            label: "Estimate Depth",
            menu: &[],
            shortcut: None,
            params: r#"{"channel":name="Depth"} → {channel,width,height,ms}: estimates relative depth of the visible image into an alpha channel (white = near); replaces a channel of that name. Needs the depth model (depth.downloadModel)"#,
            enabled: enabled_estimate,
            run: estimate_depth,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn session(w: u32, h: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h})).unwrap();
        s
    }

    /// A fake estimator: depth grows to the right, on a coarse grid like the model's output.
    fn fake() -> Estimator {
        Arc::new(|_rgb, _w, _h| {
            let (mw, mh) = (14u32, 10u32);
            Ok(DepthMap { width: mw, height: mh, data: (0..mw * mh).map(|i| (i % mw) as f32 / (mw - 1) as f32).collect() })
        })
    }

    fn channels(s: &Session) -> usize {
        s.active().unwrap().doc.channels.len()
    }

    #[test]
    fn estimates_into_a_channel_replaces_it_and_is_one_undo_step() {
        let mut s = session(64, 48);
        s.depth.estimator = Some(fake());
        let r = s.execute("image.estimateDepth", json!({})).unwrap();
        assert_eq!((r["channel"].clone(), r["width"].clone(), r["height"].clone()), (json!(0), json!(64), json!(48)));
        assert!(r["ms"].is_u64());
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.channels[0].name, "Depth");
        let v = doc.channels[0].surface.read_region(doc.bounds());
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
        // Near (white) on the right, far on the left.
        let (left, right) = (v[24 * 64 + 1], v[24 * 64 + 62]);
        assert!(right > left + 0.5, "left {left}, right {right}");
        // A second run replaces the channel in place.
        let r = s.execute("image.estimateDepth", json!({"channel": "Depth"})).unwrap();
        assert_eq!((r["channel"].clone(), channels(&s)), (json!(0), 1));
        // A different name adds one.
        assert_eq!(s.execute("image.estimateDepth", json!({"channel": "Far"})).unwrap()["channel"], 1);
        // One undo step each.
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(channels(&s), 1);
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(channels(&s), 0);
    }

    #[test]
    fn lens_blur_reads_the_depth_channel() {
        let mut s = session(64, 48);
        s.depth.estimator = Some(fake());
        s.execute("image.estimateDepth", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        s.execute("filter.blur.lensBlur", json!({"radius": 4, "depthMap": "channel", "depthChannel": "Depth", "focalDistance": 255})).unwrap();
    }

    #[test]
    fn missing_model_and_missing_injection_fail_gracefully() {
        let mut s = session(16, 16);
        // Not injected: status works, everything else errors.
        let st = s.execute("depth.modelStatus", json!({})).unwrap();
        assert_eq!((st["supported"].clone(), st["installed"].clone(), st["downloading"].clone()), (json!(false), json!(false), Value::Null));
        for (id, p) in [
            ("depth.downloadModel", json!({})),
            ("depth.cancelDownload", json!({})),
            ("depth.loadModelFiles", json!({"dinov2": "/nope", "head": "/nope"})),
            ("image.estimateDepth", json!({})),
        ] {
            assert!(s.execute(id, p).is_err(), "{id}");
        }
        // A folder without the model.
        let dir = tempfile::tempdir().unwrap();
        s.depth.models_dir = Some(dir.path().to_path_buf());
        let e = s.execute("image.estimateDepth", json!({})).unwrap_err().to_string();
        assert!(e.contains("depth model not installed: run depth.downloadModel"), "{e}");
        assert_eq!(channels(&s), 0);
    }

    #[test]
    fn hostile_params_are_errors() {
        let mut s = session(16, 16);
        s.depth.estimator = Some(fake());
        let long = "x".repeat(256);
        for p in [
            json!({"channel": ""}),
            json!({"channel": "  "}),
            json!({"channel": long}),
            json!({"channel": 5}),
            json!({"channel": "a\nb"}),
            json!([]),
            json!("Depth"),
        ] {
            assert!(s.execute("image.estimateDepth", p.clone()).is_err(), "{p}");
        }
        for id in ["depth.modelStatus"] {
            for p in [json!({"x": 1}), json!([]), json!("a")] {
                assert!(s.execute(id, p).is_err(), "{id}");
            }
        }
        assert_eq!(channels(&s), 0);
        // A bad map from the estimator is an error, not a panic or a channel.
        for map in [
            DepthMap { width: 4, height: 4, data: vec![0.5; 3] },
            DepthMap { width: 0, height: 0, data: vec![] },
            DepthMap { width: 2, height: 2, data: vec![f32::NAN; 4] },
        ] {
            s.depth.estimator = Some(Arc::new(move |_, _, _| Ok(map.clone())));
            assert!(s.execute("image.estimateDepth", json!({})).is_err());
        }
        s.depth.estimator = Some(Arc::new(|_, _, _| Err("boom".into())));
        assert!(s.execute("image.estimateDepth", json!({})).is_err());
        assert_eq!(channels(&s), 0);
    }

    /// A fake download that writes sparse files of the pinned sizes.
    fn fake_fetch() -> FetchFn {
        Box::new(|dir: PathBuf, _stop| {
            for w in &WEIGHTS {
                std::fs::File::create(dir.join(w.name)).and_then(|f| f.set_len(w.size)).map_err(|e| e.to_string())?;
            }
            let (tx, rx) = mpsc::channel();
            let total = WEIGHTS.iter().map(|w| w.size).sum();
            tx.send(FetchEvent::Progress { done: total, total }).map_err(|e| e.to_string())?;
            tx.send(FetchEvent::Done(Ok(()))).map_err(|e| e.to_string())?;
            Ok(rx)
        })
    }

    #[test]
    fn a_download_installs_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new();
        s.depth.models_dir = Some(dir.path().join("models"));
        s.depth.fetch = Some(fake_fetch());
        assert_eq!(s.execute("depth.modelStatus", json!({})).unwrap()["installed"], false);
        s.execute("depth.downloadModel", json!({})).unwrap();
        let st = s.execute("depth.modelStatus", json!({})).unwrap();
        assert_eq!(
            (st["supported"].clone(), st["installed"].clone(), st["downloading"].clone(), st["error"].clone()),
            (json!(true), json!(true), Value::Null, Value::Null)
        );
        // Installed: nothing left to download.
        assert!(s.execute("depth.downloadModel", json!({})).is_err());
        // Files of the wrong size are not installed, whatever the marker says.
        std::fs::File::create(dir.path().join("models").join(WEIGHTS[0].name)).unwrap();
        assert_eq!(s.execute("depth.modelStatus", json!({})).unwrap()["installed"], false);
        assert!(s.journal.is_empty());
    }

    #[test]
    fn a_download_can_be_cancelled_and_failures_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new();
        s.depth.models_dir = Some(dir.path().to_path_buf());
        type Slot = Arc<std::sync::Mutex<Option<(mpsc::Sender<FetchEvent>, Arc<AtomicBool>)>>>;
        let slot: Slot = Arc::default();
        let held = slot.clone();
        s.depth.fetch = Some(Box::new(move |_, stop| {
            let (tx, rx) = mpsc::channel();
            *held.lock().unwrap() = Some((tx, stop));
            Ok(rx)
        }));
        s.execute("depth.downloadModel", json!({})).unwrap();
        let (tx, stop) = slot.lock().unwrap().take().unwrap();
        tx.send(FetchEvent::Progress { done: 10, total: 100 }).unwrap();
        let st = s.execute("depth.modelStatus", json!({})).unwrap();
        assert_eq!(st["downloading"], json!({"done": 10, "total": 100}));
        assert!(s.execute("depth.downloadModel", json!({})).is_err(), "already downloading");
        assert!(s.execute("depth.loadModelFiles", json!({"dinov2": "a", "head": "b"})).is_err());
        s.execute("depth.cancelDownload", json!({})).unwrap();
        assert!(stop.load(Ordering::SeqCst));
        tx.send(FetchEvent::Done(Err("cancelled".into()))).unwrap();
        let st = s.execute("depth.modelStatus", json!({})).unwrap();
        assert_eq!((st["downloading"].clone(), st["error"].clone()), (Value::Null, Value::Null));
        // A failure is reported; a worker that vanishes is too.
        s.depth.fetch = Some(Box::new(|_, _| {
            let (tx, rx) = mpsc::channel();
            tx.send(FetchEvent::Done(Err("offline".into()))).map_err(|e| e.to_string())?;
            Ok(rx)
        }));
        s.execute("depth.downloadModel", json!({})).unwrap();
        assert_eq!(s.execute("depth.modelStatus", json!({})).unwrap()["error"], "offline");
        s.depth.fetch = Some(Box::new(|_, _| Ok(mpsc::channel().1)));
        s.execute("depth.downloadModel", json!({})).unwrap();
        assert!(s.execute("depth.modelStatus", json!({})).unwrap()["error"].is_string());
        s.depth.fetch = Some(Box::new(|_, _| Err("no thread".into())));
        assert!(s.execute("depth.downloadModel", json!({})).is_err());
    }

    #[test]
    fn load_model_files_checks_size_and_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new();
        s.depth.models_dir = Some(dir.path().join("models"));
        let small = dir.path().join("small.bin");
        std::fs::write(&small, b"not a model").unwrap();
        // The right size but the wrong content: a sparse file of zeros.
        let zeros = |w: &WeightFile| {
            let p = dir.path().join(w.name);
            std::fs::File::create(&p).unwrap().set_len(w.size).unwrap();
            p
        };
        let (d, h) = (zeros(&WEIGHTS[0]), zeros(&WEIGHTS[1]));
        let to = |p: &Path| p.to_str().unwrap().to_string();
        for p in [
            json!({"dinov2": to(&small), "head": to(&h)}),
            json!({"dinov2": to(&d), "head": to(&h)}),
            json!({"dinov2": to(&dir.path().join("missing")), "head": to(&h)}),
            json!({"dinov2": "", "head": to(&h)}),
            json!({"dinov2": to(dir.path()), "head": to(&h)}),
            json!({"head": to(&h)}),
            json!({"dinov2": 3, "head": 4}),
            json!([]),
        ] {
            assert!(s.execute("depth.loadModelFiles", p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.execute("depth.modelStatus", json!({})).unwrap()["installed"], false);
        // Nothing half-installed is left behind.
        let left: Vec<_> = std::fs::read_dir(dir.path().join("models")).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn planes_resize_and_reduce() {
        assert_eq!(fit(8000, 4000, 1024), (1024, 512));
        assert_eq!(fit(10, 20, 1024), (10, 20));
        assert_eq!(fit(10_000, 1, 100), (100, 1));
        let up = resize_plane(&[0.0, 1.0], 2, 1, 5, 3);
        assert_eq!(up.len(), 15);
        assert!(up.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(up[0] <= up[4]);
        let mut a = Accum::new(4, 2, 1, 2);
        for y in 0..2 {
            a.add_row(y, &[0.0, 0.0, 1.0, 1.0]);
        }
        assert_eq!(a.finish(), vec![0.0, 1.0]);
    }
}
