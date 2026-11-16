//! Object Selection's model: the shared Craft AI layer's `select-subject` tool (SAM 2.1 on the
//! built-in rten engine), seen by the engine as a [`photocraft_engine::object_model::ObjectModel`].
//!
//! Which model runs, and its files, are chosen in the ArtCraft Launcher (Set up AI › Select
//! subject); PhotoCraft names none. Only a model on this computer is used: the picture is never
//! sent elsewhere, and a model that would is reported as unavailable so Object Selection falls
//! back to its classical algorithm.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use craft_ai_client::{AppTools, Plan, RoleStatus};
use craft_ai_core::segment::{self, Area, Click};
use craft_ai_core::{ConsentLedger, InputRole, Locality, Output, Payload, Request};
use photocraft_engine::object_model::{ObjectModel, SegmentAnswer, SegmentRequest};

/// The catalogue id of the tool.
const TOOL: &str = "select-subject";
/// The lower-case app name the launcher scopes choices and consent by.
const APP: &str = "photocraft";
/// How long a status answer is reused (the store is read on every status call).
const STATUS_TTL: Duration = Duration::from_secs(2);
/// How often the cancel flag is looked at while the model runs.
const WATCH_STEP: Duration = Duration::from_millis(30);
/// Most bytes of a mask PNG's pixels read back (a 2048 px picture at 16 bits is 8 MiB).
const MASK_LIMIT_BYTES: usize = 64 << 20;

/// What the launcher says when the model would send the picture elsewhere.
const REMOTE_REFUSAL: &str =
    "Object Selection only uses a model on this computer. The model chosen for Select subject sends pictures elsewhere; choose a built-in model in Set up AI.";

/// The model Object Selection runs on, or `None` when the shared store cannot be opened (no home
/// folder), in which case Object Selection uses its classical algorithm.
pub fn model() -> Option<Arc<dyn ObjectModel>> {
    match SharedObjectModel::standard() {
        Ok(m) => Some(Arc::new(m)),
        Err(why) => {
            log::warn!("Object Selection has no AI model: {why}");
            None
        }
    }
}

/// [`ObjectModel`] over the shared `select-subject` tool.
pub struct SharedObjectModel {
    tools: AppTools,
    status: Mutex<Option<(Instant, Option<String>)>>,
}

impl SharedObjectModel {
    /// Over the real shared store.
    pub fn standard() -> Result<Self, String> {
        Ok(Self::with_tools(AppTools::standard(APP)?))
    }

    /// Over `tools` (tests build them over a store of their own).
    pub fn with_tools(tools: AppTools) -> Self {
        Self { tools, status: Mutex::new(None) }
    }

    fn fresh_status(&self) -> Option<String> {
        status_reason(self.tools.status(TOOL))
    }
}

/// Why `status` cannot run Object Selection, in words for a person; `None` when it can.
fn status_reason(status: RoleStatus) -> Option<String> {
    match status {
        RoleStatus::Unavailable { reason } => Some(reason),
        RoleStatus::Ready(ready) if ready.locality == Locality::Remote => Some(REMOTE_REFUSAL.to_owned()),
        RoleStatus::Ready(_) => None,
    }
}

impl ObjectModel for SharedObjectModel {
    fn unavailable(&self) -> Option<String> {
        let mut cached = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, answer)) = cached.as_ref()
            && at.elapsed() < STATUS_TTL
        {
            return answer.clone();
        }
        let answer = self.fresh_status();
        *cached = Some((Instant::now(), answer.clone()));
        answer
    }

    fn segment(&self, req: &SegmentRequest, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<SegmentAnswer, String> {
        if cancelled() {
            return Err("Cancelled.".to_owned());
        }
        let png = encode_rgb_png(&req.rgb, req.width, req.height)?;
        let base = self.tools.request(TOOL).ok_or_else(|| "The Select subject tool is not available.".to_owned())?;
        let request = build_request(base, png, req)?;
        let plan = self.tools.plan(TOOL, request)?;
        let ledger = ConsentLedger::default();
        refuse_off_computer(&plan, &ledger)?;
        let outputs = run_watched(&self.tools, &plan, &ledger, cancelled)?;
        answer_from_outputs(&outputs, req.width, req.height)
    }
}

/// Runs `plan`, asking the job to stop when `cancelled` turns true.
fn run_watched(tools: &AppTools, plan: &Plan, ledger: &ConsentLedger, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Output>, String> {
    let canceller = tools.canceller();
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let watcher = std::thread::Builder::new().name("photocraft-object-watch".into()).spawn_scoped(scope, || {
            while !done.load(Ordering::Acquire) {
                if cancelled() {
                    // The job may not be registered yet; keep asking until the run returns.
                    let _ = canceller.cancel();
                }
                std::thread::sleep(WATCH_STEP);
            }
        });
        let out = tools.run(plan, ledger);
        done.store(true, Ordering::Release);
        if let Ok(handle) = watcher {
            // A watcher that panicked has nothing to report; the run's own result stands.
            let _ = handle.join();
        }
        out
    })
}

/// Only a model on this computer: a plan that sends the picture elsewhere, or that needs consent
/// (a local model needs none), is refused.
fn refuse_off_computer(plan: &Plan, ledger: &ConsentLedger) -> Result<(), String> {
    if plan.disclosure().locality == Locality::Remote || plan.needs_consent(ledger) {
        return Err(REMOTE_REFUSAL.to_owned());
    }
    Ok(())
}

/// `base` with the picture as its source input, the box and the clicks as its parameters.
fn build_request(base: Request, png: Vec<u8>, req: &SegmentRequest) -> Result<Request, String> {
    let picture = Payload::inline("image/png", png).map_err(|e| e.message)?;
    let mut request = base.with_input(InputRole::Source, picture);
    if let Some([x0, y0, x1, y1]) = req.area {
        request = segment::with_area(request, Area { x0, y0, x1, y1 });
    }
    if !req.clicks.is_empty() {
        let clicks: Vec<Click> = req.clicks.iter().map(|&(x, y, positive)| Click { x, y, positive }).collect();
        request = segment::with_clicks(request, &clicks);
    }
    Ok(request)
}

/// Straight 8-bit RGB (`width * height * 3` bytes) as a PNG.
fn encode_rgb_png(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let expected = u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(3));
    if width == 0 || height == 0 || expected != Some(rgb.len() as u64) {
        return Err("The picture for the model has the wrong size.".to_owned());
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| format!("Could not encode the picture: {e}"))?;
    writer.write_image_data(rgb).map_err(|e| format!("Could not encode the picture: {e}"))?;
    writer.finish().map_err(|e| format!("Could not encode the picture: {e}"))?;
    Ok(out)
}

/// A mask PNG as 8-bit gray, `width * height` bytes. Gray, gray with alpha, RGB and RGBA are read
/// (colour as its luma, alpha ignored), 16-bit by its high byte; a mask of another size is refused.
fn decode_mask(png_bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let wrong = |why: &str| format!("The model's mask is not usable: {why}.");
    let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
    decoder.set_limits(png::Limits { bytes: MASK_LIMIT_BYTES });
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| wrong(&e.to_string()))?;
    let size = reader.info().size();
    if size != (width, height) {
        return Err(wrong("it is not the size of the picture"));
    }
    let len = reader.output_buffer_size().ok_or_else(|| wrong("it is too large"))?;
    if len > MASK_LIMIT_BYTES {
        return Err(wrong("it is too large"));
    }
    let mut buf = vec![0u8; len];
    let info = reader.next_frame(&mut buf).map_err(|e| wrong(&e.to_string()))?;
    let (channels, bytes) = match (info.color_type, info.bit_depth) {
        (png::ColorType::Grayscale, png::BitDepth::Eight) => (1, 1),
        (png::ColorType::Grayscale, png::BitDepth::Sixteen) => (1, 2),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => (2, 1),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Sixteen) => (2, 2),
        (png::ColorType::Rgb, png::BitDepth::Eight) => (3, 1),
        (png::ColorType::Rgb, png::BitDepth::Sixteen) => (3, 2),
        (png::ColorType::Rgba, png::BitDepth::Eight) => (4, 1),
        (png::ColorType::Rgba, png::BitDepth::Sixteen) => (4, 2),
        _ => return Err(wrong("its pixel format is not supported")),
    };
    let (w, h) = (width as usize, height as usize);
    let pixel = channels * bytes;
    let used = buf.get(..info.buffer_size()).ok_or_else(|| wrong("it is truncated"))?;
    let mut out = Vec::with_capacity(w.saturating_mul(h));
    for row in 0..h {
        let start = row.checked_mul(info.line_size).ok_or_else(|| wrong("it is too large"))?;
        let line = used.get(start..start.saturating_add(w.saturating_mul(pixel))).ok_or_else(|| wrong("it is truncated"))?;
        for px in line.chunks_exact(pixel) {
            // The high byte of each sample (the only byte of an 8-bit one).
            let sample = |c: usize| u32::from(px.get(c * bytes).copied().unwrap_or(0));
            let gray = if channels >= 3 { (sample(0) * 299 + sample(1) * 587 + sample(2) * 114 + 500) / 1000 } else { sample(0) };
            out.push(u8::try_from(gray.min(255)).unwrap_or(255));
        }
    }
    Ok(out)
}

/// The model's answer from the tool's outputs: the mask at the picture's size, and whether
/// anything matched (the tool's own word when it gave one, else whether the mask has a selected
/// pixel).
fn answer_from_outputs(outputs: &[Output], width: u32, height: u32) -> Result<SegmentAnswer, String> {
    let Some(Payload::Inline { data, .. }) = segment::mask(outputs) else {
        return Err("The model gave no mask.".to_owned());
    };
    let mask = decode_mask(data, width, height)?;
    let matched = match segment::read_found(outputs) {
        Some(found) => found.matched,
        None => mask.iter().any(|v| *v >= 128),
    };
    Ok(SegmentAnswer { mask, matched })
}

#[cfg(test)]
mod tests;
