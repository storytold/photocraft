//! `image.estimateDepth`: relative depth of the visible image into an alpha channel.
//!
//! The model is not PhotoCraft's: the work is done by the shared Craft AI layer's `estimate-depth`
//! tool (see [`ai`]), which owns the model files and their setup (Set up AI). This command sends
//! it the picture, refines the 16-bit gray picture it returns so edges follow the photo, and writes
//! the channel as one undo step. On the web build the tool does not exist and the command is
//! disabled.
// The web build has no tool, so most of the reduction code below is unused there.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use std::sync::{Arc, Mutex, PoisonError};

use photocraft_doc::{AlphaChannel, DocId, Document};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::channel_cmds::channel_format;
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

#[cfg(not(target_arch = "wasm32"))]
pub mod ai;
#[cfg(not(target_arch = "wasm32"))]
pub use ai::{AiHost, AiState, Estimator, TOOL};

/// Longest side of the picture sent to the tool (the tool resizes it again to its own sizes).
const SEND_SIDE: usize = 2048;
/// Most pixels of document the command reads.
const MAX_DOC_PIXELS: u64 = 1 << 27;
/// Most floats of full-precision plane the session keeps (the refined plane is at most 4096 x 4096).
const MAX_PRECISE_FLOATS: usize = 1 << 26;
/// Longest side of the grid the depth is refined on, before it is stretched to the document.
const REFINE_SIDE: usize = 4096;
const MAX_NAME: usize = 255;
/// Said where the tool does not exist.
pub const UNAVAILABLE_ON_WEB: &str = "Estimate Depth is not available in the browser";

/// How carefully `image.estimateDepth` works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Quality {
    /// One network pass over the whole image.
    Fast,
    /// A global pass plus overlapping full-resolution tiles (slower, finer folds and edges).
    High,
}

impl Quality {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Quality::Fast => "fast",
            Quality::High => "high",
        }
    }
}

/// The full-precision depth plane of the last estimate and what it belongs to.
struct PreciseDepth {
    doc: DocId,
    channel: String,
    /// The channel as written, to tell later whether it was edited since.
    surface: Surface,
    /// The document's size at the estimate: a canvas resize can leave the channel's surface equal
    /// while the plane no longer covers the same pixels.
    size: Size,
    plane: Arc<Vec<f32>>,
    width: u32,
    height: u32,
}

/// The float depth plane written by the last `image.estimateDepth`, if the channel `channel_name`
/// of document `doc` still is exactly what that command wrote.
///
/// Contract: `current_channel_surface` is the channel's surface as it is now; it is compared with
/// the surface the estimate wrote (`Surface ==`, which is cheap while the tiles are shared). Any
/// edit, a replaced channel, another document or another name gives `None`, so the plane is never
/// returned for pixels it does not describe. On a hit the result is `(plane, width, height)`:
/// row-major values in 0..1 (larger = nearer) before the channel's 8/16-bit quantisation. The
/// plane is the *refined* grid of the estimate: the document's size when its longer side is at most
/// 4096 px, otherwise the document scaled down to a 4096 px longer side (map document pixels onto
/// it by their fraction of the width and height). It is never more than 2^24 floats.
pub fn precise_depth_plane(s: &Session, doc: DocId, channel_name: &str, current_channel_surface: &Surface) -> Option<(Arc<Vec<f32>>, u32, u32)> {
    let guard = s.depth.precise.lock().unwrap_or_else(PoisonError::into_inner);
    let p = guard.as_ref()?;
    let current = s.documents().iter().find(|d| d.doc.id == doc)?;
    let same = p.doc == doc && p.channel == channel_name && p.size == current.doc.size && p.surface == *current_channel_surface;
    let sized = p.plane.len() == (p.width as usize).saturating_mul(p.height as usize);
    (same && sized).then(|| (p.plane.clone(), p.width, p.height))
}

#[derive(Default)]
pub struct DepthState {
    /// The shared AI layer: the tool, its status, and the hook that replaces it in tests.
    #[cfg(not(target_arch = "wasm32"))]
    pub ai: ai::AiState,
    /// Shared with the estimate job, which fills it from its worker thread.
    precise: Arc<Mutex<Option<PreciseDepth>>>,
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

fn quality_param(p: &Value) -> Result<Quality> {
    match p.get("quality") {
        None => Ok(Quality::High),
        Some(Value::String(q)) if q == "fast" => Ok(Quality::Fast),
        Some(Value::String(q)) if q == "high" => Ok(Quality::High),
        Some(_) => Err(bad("`quality` must be \"fast\" or \"high\"")),
    }
}

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

/// Column taps of a bilinear resize from `sw` to `dw` columns: (left, right, weight).
fn column_taps(sw: usize, dw: usize) -> Vec<(usize, usize, f32)> {
    let fx = sw as f32 / dw as f32;
    (0..dw)
        .map(|x| {
            let u = ((x as f32 + 0.5) * fx - 0.5).clamp(0.0, sw.saturating_sub(1) as f32);
            let x0 = u.floor() as usize;
            (x0, (x0 + 1).min(sw.saturating_sub(1)), u - x0 as f32)
        })
        .collect()
}

/// Rows `rows` of the bilinear resize of one plane (pixel centres aligned), appended to `out`.
/// `taps` is [`column_taps`]`(sw, dw)`. A source that is too small gives zeros.
fn resize_rows(
    src: &[f32],
    (sw, sh): (usize, usize),
    (dw, dh): (usize, usize),
    taps: &[(usize, usize, f32)],
    rows: std::ops::Range<usize>,
    out: &mut Vec<f32>,
) {
    if sw == 0 || sh == 0 || src.len() < sw.saturating_mul(sh) {
        out.resize(out.len() + dw * rows.len(), 0.0);
        return;
    }
    let fy = sh as f32 / dh as f32;
    for y in rows {
        let v = ((y as f32 + 0.5) * fy - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = v.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let wy = v - y0 as f32;
        let (Some(r0), Some(r1)) = (src.get(y0 * sw..(y0 + 1) * sw), src.get(y1 * sw..(y1 + 1) * sw)) else {
            out.resize(out.len() + dw, 0.0);
            continue;
        };
        out.extend(taps.iter().map(|&(x0, x1, wx)| {
            let a = r0[x0] + (r0[x1] - r0[x0]) * wx;
            let b = r1[x0] + (r1[x1] - r1[x0]) * wx;
            a + (b - a) * wy
        }));
    }
}

/// Bilinear resize of one plane (pixel centres aligned).
fn resize_plane(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    if (sw, sh) == (dw, dh) {
        return src.to_vec();
    }
    let mut out = Vec::with_capacity(dw * dh);
    resize_rows(src, (sw, sh), (dw, dh), &column_taps(sw, dw), 0..dh, &mut out);
    out
}

/// Rows of the channel written per band (the document-size plane is never held whole).
const BAND_ROWS: usize = 128;

/// The channel surface for `rect` made from the refined grid `refined` (`rw` x `rh`) stretched to
/// the document, one band of rows at a time: the same values as `plane_surface` of
/// `resize_plane(..)` without a document-size float plane.
fn refined_surface(refined: &[f32], (rw, rh): (usize, usize), rect: Rect, fmt: photocraft_color::PixelFormat) -> Surface {
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let mut surface = Surface::new(fmt);
    let taps = column_taps(rw, w.max(1));
    let mut band = Vec::with_capacity(w * BAND_ROWS.min(h));
    let mut y = 0;
    while y < h {
        let end = (y + BAND_ROWS).min(h);
        band.clear();
        if (rw, rh) == (w, h) {
            band.extend_from_slice(refined.get(y * w..end * w).unwrap_or(&[]));
            band.resize(w * (end - y), 0.0);
        } else {
            resize_rows(refined, (rw, rh), (w, h), &taps, y..end, &mut band);
        }
        for v in &mut band {
            *v = v.clamp(0.0, 1.0);
        }
        // `y` is below `h`, which fits the document's `i32` bounds.
        let (y0, y1) = (rect.y0.saturating_add(y as i32), rect.y0.saturating_add(end as i32));
        surface.write_region(Rect::new(rect.x0, y0, rect.x1, y1), &band);
        y = end;
    }
    surface.prune();
    surface
}

fn channel_name(p: &Value) -> Result<String> {
    let obj = match p {
        Value::Null => None,
        Value::Object(o) => Some(o),
        _ => return Err(bad("expected {\"channel\": name, \"quality\": \"fast\"|\"high\"}")),
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
    #[cfg(target_arch = "wasm32")]
    return Err(UNAVAILABLE_ON_WEB.to_owned());
    #[cfg(not(target_arch = "wasm32"))]
    match s.depth.ai.known_status_reason() {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

/// A plane set reduced from the composite of the visible image: (pixels, (width, height)). The RGB
/// goes to the tool, the luma guides the refinement.
type Reduced = (Vec<f32>, (usize, usize));

fn read_image(doc: &Document, ctx: &crate::jobs::JobCtx) -> Result<(Reduced, Reduced)> {
    let rect = doc.bounds();
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    if w == 0 || h == 0 {
        return Err(bad("the document is empty"));
    }
    if (w as u64).saturating_mul(h as u64) > MAX_DOC_PIXELS {
        return Err(bad("the image is too large for depth estimation"));
    }
    let fmt = photocraft_color::PixelFormat::RGBA8;
    let flat = ctx
        .stage(0.0, 0.15, "Reading image", |ctl| photocraft_compose::composite_to_surface_with_interrupt(doc, rect, fmt, Some([1.0; 3]), ctl))
        .map_err(|_| EngineError::Cancelled)?;
    let mut rgb = Accum::new(w, h, 3, SEND_SIDE);
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
            for (x, p) in line.as_chunks::<4>().0.iter().enumerate() {
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
    Ok(((rgb.finish(), rd), (luma.finish(), ld)))
}

/// What an estimate produced.
struct Estimated {
    /// The refined plane (0..1), before it is stretched to the document, and its size.
    refined: Vec<f32>,
    refined_size: (usize, usize),
}

/// Send the picture to the tool and refine its map to the document size: edges follow the photo.
#[cfg(not(target_arch = "wasm32"))]
fn estimate_plane(doc: &Document, ctx: &crate::jobs::JobCtx, backend: &ai::Backend, quality: Quality) -> Result<Estimated> {
    let ((rgb, (rw, rh)), (luma, (lw, lh))) = read_image(doc, ctx)?;
    let bytes: Vec<u8> = rgb.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect();
    drop(rgb);
    let (rw32, rh32) = (u32::try_from(rw).map_err(|_| bad("the image is too large"))?, u32::try_from(rh).map_err(|_| bad("the image is too large"))?);
    let png = ai::encode_rgb8(&bytes, rw32, rh32)?;
    drop(bytes);
    ctx.progress(0.15, "Estimating depth");
    let answer = backend(ctx, png, rw32, rh32, quality)?;
    if ctx.cancelled() {
        return Err(EngineError::Cancelled);
    }
    let map = ai::decode_gray(&answer, rw32, rh32)?;
    drop(answer);
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
    Ok(Estimated { refined, refined_size: (lw, lh) })
}

#[cfg(target_arch = "wasm32")]
fn estimate_depth(s: &mut Session, p: &Value) -> Result<Value> {
    channel_name(p)?;
    quality_param(p)?;
    s.active().ok_or(EngineError::NoDocument)?;
    Err(bad(UNAVAILABLE_ON_WEB))
}

#[cfg(not(target_arch = "wasm32"))]
fn estimate_depth(s: &mut Session, p: &Value) -> Result<Value> {
    let name = channel_name(p)?;
    let quality = quality_param(p)?;
    s.active().ok_or(EngineError::NoDocument)?;
    if let Some(reason) = s.depth.ai.status_reason_now() {
        return Err(bad(reason));
    }
    let backend = s.depth.ai.backend().map_err(bad)?;
    let precise = s.depth.precise.clone();
    let started = web_time::Instant::now();
    crate::jobs::edit_job(
        s,
        "Estimate Depth",
        move |doc, _, ctx| {
            let est = estimate_plane(doc, ctx, &backend, quality)?;
            let rect = doc.bounds();
            let surface = refined_surface(&est.refined, est.refined_size, rect, channel_format(doc));
            let snapshot = surface.clone();
            let index = match doc.channels.iter().position(|c| c.name == name) {
                Some(i) => {
                    doc.channels[i].surface = surface;
                    i
                }
                None => {
                    doc.channels.push(AlphaChannel::new(name.clone(), surface));
                    doc.channels.len() - 1
                }
            };
            let (pw, ph) = est.refined_size;
            let keep = pw.saturating_mul(ph) <= MAX_PRECISE_FLOATS && est.refined.len() == pw * ph;
            let entry = keep.then(|| PreciseDepth {
                doc: doc.id,
                channel: name,
                surface: snapshot,
                size: doc.size,
                plane: Arc::new(est.refined),
                width: pw as u32,
                height: ph as u32,
            });
            Ok((index, rect.width(), rect.height(), entry))
        },
        // Only a committed edit replaces the kept plane: a cancelled job or a failed commit leaves
        // the previous one, which still matches its channel.
        move |(index, w, h, entry)| {
            *precise.lock().unwrap_or_else(PoisonError::into_inner) = entry;
            json!({"channel": index, "width": w, "height": h, "quality": quality.name(), "ms": started.elapsed().as_millis() as u64})
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "image.estimateDepth",
        label: "Estimate Depth",
        menu: &[],
        shortcut: None,
        params: r#"{"channel":name="Depth","quality":"fast"|"high"="high"} → {channel,width,height,quality,ms}: estimates relative depth of the visible image into an alpha channel (white = near); replaces a channel of that name. Runs the shared Craft AI tool `estimate-depth` (on this computer; nothing leaves it); "high" adds overlapping full-resolution tiles (slower, much finer folds and edges). Disabled with the reason while the tool is not set up: set it up with Set up AI (the ArtCraft Launcher, or Estimate Depth in the brush settings). Not available in the browser"#,
        enabled: enabled_estimate,
        run: estimate_depth,
        journal: true,
    }]
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::channel_cmds::plane_surface;
    use craft_ai_client::AiClient;
    use craft_ai_core::AppId;
    use craft_ai_native::secrets::MemoryStore;
    use craft_ai_native::shared::SharedStore;

    /// A host over an empty temporary store (nothing set up) so no test reads a person's settings.
    fn empty_host(dir: &std::path::Path) -> AiHost {
        let store = SharedStore::new(dir.join("connections.json"));
        let shared = store.clone();
        AiHost::new(store, move || {
            let app = AppId::new("photocraft").map_err(|e| e.to_string())?;
            Ok(AiClient::new(app, shared.clone(), Arc::new(MemoryStore::new())))
        })
    }

    fn session(w: u32, h: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h})).unwrap();
        s
    }

    /// A fake tool: depth grows to the right, on a coarse grid like a model's output, at the size
    /// of the picture it was given. Records the quality it was asked for.
    fn fake(seen: Arc<Mutex<Vec<String>>>) -> Estimator {
        Arc::new(move |png, w, h, quality| {
            seen.lock().unwrap().push(quality.to_owned());
            // What was sent is a PNG of the size it says.
            let dec = png::Decoder::new(std::io::Cursor::new(png)).read_info().map_err(|e| e.to_string())?;
            assert_eq!((dec.info().width, dec.info().height, dec.info().color_type), (w, h, png::ColorType::Rgb));
            let data: Vec<u16> = (0..w * h).map(|i| ((i % w) as f32 / (w - 1).max(1) as f32 * 65535.0) as u16).collect();
            Ok(ai::encode_gray16(&data, w, h))
        })
    }

    fn fake_session(w: u32, h: u32) -> (Session, Arc<Mutex<Vec<String>>>) {
        let mut s = session(w, h);
        let seen = Arc::new(Mutex::new(Vec::new()));
        s.depth.ai.estimator = Some(fake(seen.clone()));
        (s, seen)
    }

    fn channels(s: &Session) -> usize {
        s.active().unwrap().doc.channels.len()
    }

    #[test]
    fn an_unset_tool_disables_the_command_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(16, 16);
        s.depth.ai.set_host(empty_host(dir.path()));
        let reason = s.depth.ai.status_reason_now().expect("nothing is set up");
        assert!(!reason.trim().is_empty());
        let e = s.execute("image.estimateDepth", json!({})).unwrap_err().to_string();
        assert!(e.contains(&reason), "{e} vs {reason}");
        assert_eq!(channels(&s), 0);
        // Without a document the command is off for that reason first.
        let none = Session::new();
        assert!(enabled_estimate(&none).is_err());
        // The status is reused for a moment and forgotten on request.
        assert_eq!(s.depth.ai.status_reason(), Some(reason));
        s.depth.ai.forget_status();
        assert!(s.depth.ai.status_reason().is_some());
    }

    /// The per-frame status never waits for the shared store: it answers at once and fills in the
    /// real reason from a background thread.
    #[test]
    fn the_per_frame_status_does_not_block_on_the_store() {
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let s = Session::new();
        s.depth.ai.set_host(AiHost::new(SharedStore::new("/nonexistent/x.json"), move || {
            // The slow part (key-store setup) blocks until the test lets it go.
            let _ = release_rx.lock().unwrap().recv();
            Err("no AI folder here".to_owned())
        }));
        let began = std::time::Instant::now();
        let first = s.depth.ai.status_reason().expect("provisional while checking");
        assert!(began.elapsed() < std::time::Duration::from_secs(5), "blocked for {:?}", began.elapsed());
        assert!(first.contains("Checking"), "{first}");
        // Asking again while the read runs starts nothing else and still answers at once.
        assert_eq!(s.depth.ai.status_reason(), Some(first));
        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if s.depth.ai.status_reason().as_deref() == Some("no AI folder here") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the background read never answered");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// A new host drops what was known about the old one.
    #[test]
    fn a_new_host_is_seen_at_once() {
        let s = Session::new();
        s.depth.ai.set_host(AiHost::new(SharedStore::new("/nonexistent/x.json"), || Err("old".to_owned())));
        assert_eq!(s.depth.ai.status_reason_now().as_deref(), Some("old"));
        s.depth.ai.set_host(AiHost::new(SharedStore::new("/nonexistent/x.json"), || Err("new".to_owned())));
        assert_eq!(s.depth.ai.status_reason_now().as_deref(), Some("new"));
        assert_eq!(s.depth.ai.status_reason().as_deref(), Some("new"));
    }

    #[test]
    fn a_first_call_before_any_status_read_gets_the_real_reason() {
        // No frame has read the status yet: the command must not be refused as "checking", but
        // report why the tool cannot run.
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(16, 16);
        s.depth.ai.set_host(empty_host(dir.path()));
        let e = s.execute("image.estimateDepth", json!({})).unwrap_err().to_string();
        assert!(!e.contains("Checking"), "{e}");
        let reason = s.depth.ai.status_reason_now().expect("nothing is set up");
        assert!(e.contains(&reason), "{e} vs {reason}");
        assert_eq!(channels(&s), 0);
    }

    /// Reading whether the command is enabled twice in a row gives one answer, before any status
    /// was read: a provisional "enabled" while a background read runs would flip a command listing.
    #[test]
    fn enabled_reads_on_a_fresh_session_agree() {
        let dir = tempfile::tempdir().unwrap();
        let not_set_up = session(8, 8);
        not_set_up.depth.ai.set_host(empty_host(dir.path()));
        let no_host = session(8, 8);
        for (case, s) in [("tool not set up", &not_set_up), ("no host", &no_host)] {
            let first = s.disabled_reason("image.estimateDepth");
            let second = s.disabled_reason("image.estimateDepth");
            assert!(first.is_some(), "{case}: the tool cannot run");
            assert_eq!(first, second, "{case}");
            assert!(!s.is_enabled("image.estimateDepth"), "{case}");
        }
    }

    #[test]
    fn a_host_that_cannot_be_made_is_the_reason() {
        let mut s = session(8, 8);
        s.depth.ai.set_host(AiHost::new(SharedStore::new("/nonexistent/x.json"), || Err("no AI folder here".to_owned())));
        assert_eq!(s.depth.ai.status_reason_now().as_deref(), Some("no AI folder here"));
        assert!(s.execute("image.estimateDepth", json!({})).is_err());
    }

    #[test]
    fn estimates_into_a_channel_replaces_it_and_is_one_undo_step() {
        let (mut s, seen) = fake_session(64, 48);
        let r = s.execute("image.estimateDepth", json!({})).unwrap();
        assert_eq!((r["channel"].clone(), r["width"].clone(), r["height"].clone()), (json!(0), json!(64), json!(48)));
        assert!(r["ms"].is_u64());
        assert_eq!(seen.lock().unwrap().as_slice(), ["high"]);
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
    fn quality_is_validated_and_sent_to_the_tool() {
        let (mut s, seen) = fake_session(16, 16);
        for p in [json!({"quality": "ultra"}), json!({"quality": 3}), json!({"quality": null}), json!({"quality": ["fast"]}), json!({"quality": "High"})] {
            assert!(s.execute("image.estimateDepth", p.clone()).is_err(), "{p}");
        }
        assert_eq!(channels(&s), 0);
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(s.execute("image.estimateDepth", json!({})).unwrap()["quality"], "high");
        assert_eq!(s.execute("image.estimateDepth", json!({"quality": "fast"})).unwrap()["quality"], "fast");
        assert_eq!(seen.lock().unwrap().as_slice(), ["high", "fast"]);
    }

    #[test]
    fn a_large_document_is_reduced_before_it_is_sent() {
        let (mut s, _) = fake_session(3000, 1000);
        let sizes = Arc::new(Mutex::new(Vec::new()));
        let log = sizes.clone();
        let inner = s.depth.ai.estimator.clone().unwrap();
        s.depth.ai.estimator = Some(Arc::new(move |png, w, h, q| {
            log.lock().unwrap().push((w, h));
            inner(png, w, h, q)
        }));
        let r = s.execute("image.estimateDepth", json!({})).unwrap();
        assert_eq!((r["width"].clone(), r["height"].clone()), (json!(3000), json!(1000)));
        assert_eq!(sizes.lock().unwrap().as_slice(), [(2048, 683)]);
        let doc = &s.active().unwrap().doc;
        let v = doc.channels[0].surface.read_region(doc.bounds());
        assert!(v.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x)));
    }

    #[test]
    fn the_precise_plane_is_kept_until_the_channel_is_edited() {
        let (mut s, _) = fake_session(64, 48);
        let id = s.active().unwrap().doc.id;
        s.execute("image.estimateDepth", json!({})).unwrap();
        let surface = s.active().unwrap().doc.channels[0].surface.clone();
        let (plane, w, h) = precise_depth_plane(&s, id, "Depth", &surface).expect("plane right after the estimate");
        assert_eq!((w, h, plane.len()), (64, 48, 64 * 48));
        assert!(plane.iter().all(|v| (0.0..=1.0).contains(v)));
        // Other names and documents do not match.
        assert!(precise_depth_plane(&s, id, "Other", &surface).is_none());
        assert!(precise_depth_plane(&s, DocId(id.0 + 1), "Depth", &surface).is_none());
        // Editing the channel invalidates it.
        let mut edited = surface.clone();
        let r = Rect::new(0, 0, 4, 4);
        edited.write_region(r, &[0.123; 16]);
        assert!(precise_depth_plane(&s, id, "Depth", &edited).is_none());
        // A fresh estimate replaces the plane.
        s.execute("image.estimateDepth", json!({"quality": "fast"})).unwrap();
        let now = s.active().unwrap().doc.channels[0].surface.clone();
        assert!(precise_depth_plane(&s, id, "Depth", &now).is_some());
        assert!(precise_depth_plane(&s, id, "Depth", &edited).is_none());
    }

    /// A canvas change that leaves the channel's surface equal (a top-left expansion adds nothing
    /// the surface stores) must not return a plane that covers the old extent.
    #[test]
    fn the_precise_plane_is_dropped_when_the_canvas_size_changes() {
        let (mut s, _) = fake_session(64, 48);
        let id = s.active().unwrap().doc.id;
        s.execute("image.estimateDepth", json!({})).unwrap();
        let surface = s.active().unwrap().doc.channels[0].surface.clone();
        assert!(precise_depth_plane(&s, id, "Depth", &surface).is_some());
        s.edit("Canvas", |doc, _| {
            doc.size = Size::new(128, 48);
            Ok(())
        })
        .unwrap();
        let now = s.active().unwrap().doc.channels[0].surface.clone();
        assert!(now == surface, "the channel surface is unchanged");
        assert!(precise_depth_plane(&s, id, "Depth", &now).is_none());
    }

    #[test]
    fn lens_blur_reads_the_depth_channel() {
        let (mut s, _) = fake_session(64, 48);
        s.execute("image.estimateDepth", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        s.execute("filter.blur.lensBlur", json!({"radius": 4, "depthMap": "channel", "depthChannel": "Depth", "focalDistance": 255})).unwrap();
    }

    #[test]
    fn hostile_params_are_errors() {
        let (mut s, _) = fake_session(16, 16);
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
        assert_eq!(channels(&s), 0);
    }

    #[test]
    fn a_bad_answer_from_the_tool_is_an_error_not_a_channel() {
        let (mut s, _) = fake_session(16, 16);
        let gray8 = |w: u32, h: u32, value: u8| {
            let mut out = Vec::new();
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&vec![value; (w * h) as usize]).unwrap();
            out
        };
        let rgb = ai::encode_rgb8(&[0; 16 * 16 * 3], 16, 16).unwrap();
        let answers: Vec<std::result::Result<Vec<u8>, String>> = vec![
            Err("boom".into()),
            Ok(Vec::new()),
            Ok(b"not a png".to_vec()),
            // The wrong size, a colour picture.
            Ok(gray8(8, 8, 3)),
            Ok(rgb),
        ];
        for a in answers {
            s.depth.ai.estimator = Some(Arc::new(move |_, _, _, _| a.clone()));
            assert!(s.execute("image.estimateDepth", json!({})).is_err());
        }
        // An 8-bit gray picture of the right size is fine.
        s.depth.ai.estimator = Some(Arc::new(move |_, _, _, _| Ok(gray8(16, 16, 200))));
        assert!(s.execute("image.estimateDepth", json!({})).is_ok());
    }

    #[test]
    fn a_running_estimate_can_be_cancelled() {
        use std::sync::mpsc;
        let mut s = session(32, 32);
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        s.depth.ai.estimator = Some(Arc::new(move |_, w, h, _| {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
            Ok(ai::encode_gray16(&vec![1000; (w * h) as usize], w, h))
        }));
        let started = s.start("image.estimateDepth", json!({})).unwrap();
        let crate::jobs::Started::Job(job) = started else { panic!("the estimate runs as a background job") };
        entered_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        assert!(s.cancel_job(job));
        let _ = release_tx.send(());
        assert_eq!(channels(&s), 0);
    }

    /// A job that does not commit (cancelled after its worker finished) leaves the plane kept for
    /// the channel as it still is.
    #[test]
    fn a_job_that_does_not_commit_keeps_the_previous_precise_plane() {
        let (mut s, _) = fake_session(32, 32);
        let id = s.active().unwrap().doc.id;
        s.execute("image.estimateDepth", json!({})).unwrap();
        let surface = s.active().unwrap().doc.channels[0].surface.clone();
        let (before, _, _) = precise_depth_plane(&s, id, "Depth", &surface).expect("plane after the first estimate");
        // A second estimate whose worker finishes, then is cancelled before its result is applied.
        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
        s.depth.ai.estimator = Some(Arc::new(move |_, w, h, _| {
            let _ = entered_tx.send(());
            Ok(ai::encode_gray16(&vec![30000; (w * h) as usize], w, h))
        }));
        let crate::jobs::Started::Job(job) = s.start("image.estimateDepth", json!({})).unwrap() else { panic!("the estimate runs as a background job") };
        entered_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        // Long enough for the worker to refine and finish; the result is not polled yet.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(s.cancel_job(job));
        let now = s.active().unwrap().doc.channels[0].surface.clone();
        let (after, _, _) = precise_depth_plane(&s, id, "Depth", &now).expect("the cancelled job did not replace the plane");
        assert_eq!(*before, *after);
    }

    /// The channel built band by band equals the one built from a full document-size plane.
    #[test]
    fn the_banded_channel_matches_the_full_plane_path() {
        let doc = session(4, 4).active().unwrap().doc.clone();
        let fmt = channel_format(&doc);
        for (rw, rh, w, h) in [(50usize, 40usize, 70usize, 300usize), (64, 48, 64, 48), (97, 301, 20, 40), (3, 5, 300, 131), (1, 1, 5, 129)] {
            let refined: Vec<f32> = (0..rw * rh).map(|i| ((i * 7919) % 1000) as f32 / 999.0).collect();
            for (x0, y0) in [(0, 0), (-13, 29)] {
                let rect = Rect::new(x0, y0, x0 + w as i32, y0 + h as i32);
                let old = plane_surface(&resize_plane(&refined, rw, rh, w, h), rect, fmt);
                let new = refined_surface(&refined, (rw, rh), rect, fmt);
                let (a, b) = (old.read_region(rect), new.read_region(rect));
                assert_eq!(a.len(), b.len());
                assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= 1.0 / 65535.0), "{rw}x{rh} -> {w}x{h} at ({x0},{y0})");
            }
        }
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

    #[test]
    fn gray_pictures_decode_to_unit_values() {
        let png = ai::encode_gray16(&[0, 65535, 32768, 1], 2, 2);
        let m = ai::decode_gray(&png, 2, 2).unwrap();
        assert_eq!((m.width, m.height), (2, 2));
        assert_eq!((m.data[0], m.data[1]), (0.0, 1.0));
        assert!((m.data[2] - 0.5).abs() < 1e-3);
        assert!(ai::decode_gray(&png, 2, 3).is_err());
    }
}
