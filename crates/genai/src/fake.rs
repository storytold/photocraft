//! CPU-only in-process native worker for command and UI tests.
use crate::native::NativeBackend;
use gn_engine::protocol::{Blob, DoneInfo, ErrorKind, Health, Preview, RunRequest};
use gn_engine::server::{Engine, EngineError};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Options {
    pub delay_ms: u64,
    pub fail: Option<String>,
    pub color: [u8; 4],
    pub echo: bool,
    pub progress_steps: u32,
    pub segments: Vec<[f32; 4]>,
    pub segments_for: Vec<(String, Vec<[f32; 4]>)>,
    pub missing_files: Vec<String>,
    pub matte: Option<[f32; 4]>,
    pub enhanced_text: Option<String>,
    /// The card's free memory as the health reports it (none: a card it cannot tell about).
    pub vram_free: Option<u64>,
    /// Answer a `batch` request with one picture, as a worker without batches would.
    pub no_batch: bool,
    /// The index lists the templates served and no per-folder files, as gn-diffusion's does.
    pub templates_only: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            delay_ms: 0,
            fail: None,
            color: [200, 40, 40, 255],
            echo: false,
            progress_steps: 4,
            segments: vec![[0.25, 0.25, 0.75, 0.75]],
            segments_for: Vec::new(),
            missing_files: Vec::new(),
            matte: None,
            enhanced_text: None,
            vram_free: None,
            no_batch: false,
            templates_only: false,
        }
    }
}
#[derive(Debug, Clone)]
pub struct RecordedRun {
    pub run: RunRequest,
    pub request: Value,
    pub blobs: Vec<Vec<u8>>,
}
impl RecordedRun {
    /// Source pixels received by the worker, without PNG conversion.
    pub fn image(&self) -> crate::Result<crate::Rgba8> {
        let b = self.run.image.as_ref().ok_or_else(|| crate::Error::Image("no input picture".into()))?;
        crate::Rgba8::new(b.width, b.height, self.blobs.get(b.index as usize).cloned().unwrap_or_default())
    }
    pub fn mask(&self) -> crate::Result<crate::Gray8> {
        let b = self.run.mask.as_ref().ok_or_else(|| crate::Error::Image("no input mask".into()))?;
        crate::Gray8::new(b.width, b.height, self.blobs.get(b.index as usize).cloned().unwrap_or_default())
    }
}
#[derive(Debug, Default)]
pub struct State {
    pub runs: Vec<RecordedRun>,
    pub pictures: Vec<crate::Rgba8>,
    pub cancellations: u32,
    pub frees: u32,
}
static BACKENDS: OnceLock<Mutex<BTreeMap<String, Weak<NativeBackend>>>> = OnceLock::new();
pub(crate) fn backend(key: &str) -> Option<Arc<NativeBackend>> {
    BACKENDS.get()?.lock().unwrap_or_else(PoisonError::into_inner).get(key)?.upgrade()
}
pub struct FakeNative {
    /// Unique model-root fixture key; never a production model folder.
    pub url: String,
    state: Arc<Mutex<State>>,
    worker: Arc<NativeBackend>,
}
impl FakeNative {
    pub fn start() -> std::io::Result<Self> {
        Self::start_with(Options::default())
    }
    pub fn start_with(options: Options) -> std::io::Result<Self> {
        let listener = gn_engine::server::bind()?;
        let port = listener.local_addr()?.port();
        let token = gn_engine::client::new_token();
        let state = Arc::new(Mutex::new(State::default()));
        let st = Arc::clone(&state);
        let t = token.clone();
        std::thread::Builder::new().name("fake-native".into()).spawn(move || {
            let mut engine = FakeEngine { options, state: st };
            let _ = gn_engine::server::serve(&listener, &t, &mut engine);
        })?;
        let worker = Arc::new(NativeBackend::attached(port, &token).map_err(std::io::Error::other)?);
        let url = format!("fake-native:{port}:{token}");
        BACKENDS.get_or_init(Default::default).lock().unwrap_or_else(PoisonError::into_inner).insert(url.clone(), Arc::downgrade(&worker));
        Ok(Self { url, state, worker })
    }
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
impl Drop for FakeNative {
    fn drop(&mut self) {
        if let Some(registry) = BACKENDS.get() {
            registry.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.url);
        }
        self.worker.stop();
    }
}
struct FakeEngine {
    options: Options,
    state: Arc<Mutex<State>>,
}
impl Engine for FakeEngine {
    fn name(&self) -> String {
        "fake".into()
    }
    fn health(&mut self) -> Health {
        Health {
            ok: true,
            engine: "fake".into(),
            device: "CPU fake".into(),
            vram_total: None,
            vram_free: self.options.vram_free,
            resident: Vec::new(),
            detail: String::new(),
        }
    }
    fn index(&mut self, dir: &str) -> Value {
        let templates = crate::template::builtin();
        let installed: Vec<_> =
            templates.iter().filter(|t| !t.meta.models.iter().any(|m| self.options.missing_files.contains(&m.default))).map(|t| &t.meta.id).collect();
        if self.options.templates_only {
            return json!({"dir":dir,"installed":installed});
        }
        let mut folders: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for t in &templates {
            for slot in &t.meta.models {
                if !self.options.missing_files.contains(&slot.default) {
                    folders.entry(slot.folder.clone()).or_default().push(slot.default.clone());
                }
            }
        }
        for files in folders.values_mut() {
            files.sort();
            files.dedup();
        }
        json!({"dir":dir,"installed":installed,"folders":folders})
    }
    fn free(&mut self) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).frees += 1;
    }
    fn run(
        &mut self,
        req: &RunRequest,
        blobs: &[Vec<u8>],
        progress: &mut dyn FnMut(f32, &str),
        cancel: &AtomicBool,
    ) -> Result<(DoneInfo, Vec<Vec<u8>>), EngineError> {
        self.run_previewed(req, blobs, progress, &mut |_| {}, cancel)
    }
    /// With `req.preview` (and a picture to make): after each step a grey picture an eighth of
    /// the result's size whose level rises with the step, as a `fast` preview arrives.
    fn run_previewed(
        &mut self,
        req: &RunRequest,
        blobs: &[Vec<u8>],
        progress: &mut dyn FnMut(f32, &str),
        preview: &mut dyn FnMut(Preview),
        cancel: &AtomicBool,
    ) -> Result<(DoneInfo, Vec<Vec<u8>>), EngineError> {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            for b in req.image.iter().chain(req.mask.iter()).chain(req.references.iter()) {
                if let Some(bytes) = blobs.get(b.index as usize) {
                    let data = if b.channels == 1 { bytes.iter().flat_map(|v| [*v, *v, *v, 255]).collect() } else { bytes.clone() };
                    if let Ok(p) = crate::Rgba8::new(b.width, b.height, data) {
                        state.pictures.push(p);
                    }
                }
            }
            state.runs.push(RecordedRun {
                run: req.clone(),
                request: serde_json::to_value(req).map_err(|e| EngineError::new(ErrorKind::Internal, e.to_string()))?,
                blobs: blobs.to_vec(),
            });
        }
        let steps = self.options.progress_steps.clamp(1, 1000);
        let start = std::time::Instant::now();
        let (pw, ph) = req.image.as_ref().map(|b| (b.width, b.height)).unwrap_or((req.width.max(1), req.height.max(1)));
        let (pw, ph) = ((pw / 8).clamp(1, 512), (ph / 8).clamp(1, 512));
        let previews = !req.preview.is_off() && req.task != "text" && req.task != "segment";
        // A batch numbers its pictures' previews by their place in the request.
        let batched = req.params.get("batch").and_then(Value::as_array).map(Vec::len).filter(|_| !self.options.no_batch).unwrap_or(1).clamp(1, 64);
        for i in 0..steps {
            while start.elapsed().as_millis() < u128::from(self.options.delay_ms) * u128::from(i + 1) / u128::from(steps) {
                if cancel.load(Ordering::SeqCst) {
                    self.state.lock().unwrap_or_else(PoisonError::into_inner).cancellations += 1;
                    return Err(EngineError::cancelled());
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            progress((i + 1) as f32 / steps as f32, "Sampling");
            if previews {
                let level = (255 * (i + 1) / steps) as u8;
                for output in 0..batched as u32 {
                    let rgba = [level, level, level, 255].repeat(pw as usize * ph as usize);
                    preview(Preview { step: i + 1, steps, output, width: pw, height: ph, rgba });
                }
            }
        }
        if cancel.load(Ordering::SeqCst) {
            self.state.lock().unwrap_or_else(PoisonError::into_inner).cancellations += 1;
            return Err(EngineError::cancelled());
        }
        if let Some(e) = &self.options.fail {
            return Err(EngineError::new(ErrorKind::Internal, e));
        }
        // A warm-up loads (the delay above) and generates nothing, as the real worker answers one.
        if req.params.get("warmup").and_then(Value::as_bool) == Some(true) {
            let free = self.options.vram_free;
            let text = match req.params.get("need_free").and_then(Value::as_u64) {
                Some(need) if free.is_none_or(|f| f < need) => {
                    format!("skipped: {:.1} GB free on the card, {:.1} GB wanted", free.unwrap_or(0) as f64 / 1e9, need as f64 / 1e9)
                }
                _ => "warm".into(),
            };
            return Ok((DoneInfo { images: Vec::new(), texts: vec![text], seed: req.seed, elapsed_ms: 0 }, Vec::new()));
        }
        if req.task == "text" {
            let prompt = req.prompt.rsplit("Instruction: ").next().unwrap_or(&req.prompt).rsplit("Idea: ").next().unwrap_or(&req.prompt);
            let text = self.options.enhanced_text.clone().unwrap_or_else(|| format!("Enhanced: {prompt}"));
            return Ok((DoneInfo { images: Vec::new(), texts: vec![text], seed: req.seed, elapsed_ms: 0 }, Vec::new()));
        }
        // A batch of edits, as the real worker answers one: each picture drawn alone.
        if !self.options.no_batch && req.params.contains_key("batch") {
            let mut descriptors = Vec::new();
            let mut pictures = Vec::new();
            for (k, b) in req.image.iter().chain(req.references.iter()).enumerate() {
                let one = RunRequest { image: Some(b.clone()), references: Vec::new(), ..req.clone() };
                let (_, mut drawn) = self.draw(&one, blobs)?;
                descriptors.push(Blob { width: b.width, height: b.height, channels: 4, index: k as u32 });
                pictures.push(drawn.swap_remove(0));
            }
            return Ok((DoneInfo { images: descriptors, texts: Vec::new(), seed: req.seed, elapsed_ms: 0 }, pictures));
        }
        let (descriptors, pictures) = self.draw(req, blobs)?;
        Ok((DoneInfo { images: descriptors, texts: Vec::new(), seed: req.seed, elapsed_ms: 0 }, pictures))
    }
}
impl FakeEngine {
    /// The pictures a run returns: one per segment or layer, else one.
    fn draw(&self, req: &RunRequest, blobs: &[Vec<u8>]) -> Result<(Vec<Blob>, Vec<Vec<u8>>), EngineError> {
        let (w, h) = req.image.as_ref().map(|b| (b.width, b.height)).unwrap_or((req.width.max(1), req.height.max(1)));
        let n = (w as usize)
            .checked_mul(h as usize)
            .filter(|n| *n <= 16 * 1024 * 1024)
            .ok_or_else(|| EngineError::new(ErrorKind::Internal, "fake picture too large"))?;
        let rects = self.options.segments_for.iter().find(|(s, _)| s == &req.prompt).map(|(_, r)| r).unwrap_or(&self.options.segments);
        let count = if req.task == "segment" {
            rects.len()
        } else if req.task == "layers" {
            req.params.get("layers").and_then(Value::as_u64).unwrap_or(3).clamp(1, 8) as usize
        } else {
            1
        };
        let mut descriptors = Vec::new();
        let mut pictures = Vec::new();
        for layer in 0..count {
            let channels = if req.task == "segment" { 1 } else { 4 };
            let mut data = Vec::with_capacity(n * channels as usize);
            let input = req.image.as_ref().and_then(|b| blobs.get(b.index as usize));
            for y in 0..h {
                for x in 0..w {
                    let inside = |r: &[f32; 4]| -> bool {
                        x as f32 >= r[0] * w as f32 && (x as f32) < r[2] * w as f32 && y as f32 >= r[1] * h as f32 && (y as f32) < r[3] * h as f32
                    };
                    if req.task == "segment" {
                        data.push(if rects.get(layer).is_some_and(inside) { 255 } else { 0 });
                        continue;
                    }
                    let mut c = self.options.color;
                    if self.options.echo
                        && let Some(src) = input.and_then(|v| v.get((y as usize * w as usize + x as usize) * 4..)).and_then(|v| v.get(..4))
                    {
                        c.copy_from_slice(src);
                    }
                    if req.task == "layers" {
                        c[3] = if y as usize * count / (h as usize) == layer { 255 } else { 0 };
                    }
                    if let Some(r) = self.options.matte {
                        c[3] = if inside(&r) {
                            let edge = x == (r[0] * w as f32) as u32
                                || x + 1 == (r[2] * w as f32) as u32
                                || y == (r[1] * h as f32) as u32
                                || y + 1 == (r[3] * h as f32) as u32;
                            if edge { 128 } else { 255 }
                        } else {
                            0
                        };
                    }
                    data.extend_from_slice(&c);
                }
            }
            descriptors.push(Blob { width: w, height: h, channels, index: layer as u32 });
            pictures.push(data);
        }
        Ok((descriptors, pictures))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GenerativeBackend, Gray8, NoProgress, Request, Rgba8};
    #[test]
    fn protocol_records_picture_mask_references_and_echoes_pixels() {
        let fake = FakeNative::start_with(Options { echo: true, ..Default::default() }).unwrap();
        let backend = backend(&fake.url).unwrap();
        let mut req = Request::new("qwen-edit-2511/fill-ref", "a red hat");
        req.image = Some(Rgba8::solid(8, 8, [3, 4, 5, 255]).unwrap());
        req.mask = Some(Gray8::new(8, 8, vec![255; 64]).unwrap());
        req.references.push(Rgba8::solid(4, 4, [7, 8, 9, 255]).unwrap());
        req.params.insert("denoise".into(), json!(0.7));
        req.seed = 17;
        let response = backend.run(&req, &NoProgress).unwrap();
        assert_eq!(response.images[0], req.image.clone().unwrap());
        let st = fake.state();
        let sent = &st.runs[0];
        assert_eq!(sent.run.model, req.template);
        assert_eq!(sent.run.seed, 17);
        assert!(sent.run.prompt.contains("a red hat"));
        assert_eq!(sent.run.params["denoise"], json!(0.7));
        assert_eq!(sent.image().unwrap(), req.image.unwrap());
        assert_eq!(sent.mask().unwrap(), req.mask.unwrap());
        assert_eq!(sent.run.references.len(), 1);
        let reference = &sent.run.references[0];
        assert_eq!(sent.blobs[reference.index as usize], req.references[0].data);
    }
    #[test]
    fn protocol_answers_matte_segments_chat_and_injected_errors() {
        let fake = FakeNative::start_with(Options { matte: Some([0.25, 0.25, 0.75, 0.75]), ..Default::default() }).unwrap();
        let backend = backend(&fake.url).unwrap();
        let mut req = Request::new("qwen-2.1/matte", "the subject");
        req.image = Some(Rgba8::solid(8, 8, [1, 2, 3, 255]).unwrap());
        let matte = backend.run(&req, &NoProgress).unwrap();
        assert_eq!(matte.images[0].get(0, 0).unwrap()[3], 0);
        assert_eq!(matte.images[0].get(4, 4).unwrap()[3], 255);
        req.template = "sam3.1/segment".into();
        let masks = backend.run(&req, &NoProgress).unwrap();
        assert_eq!(masks.images[0].get(4, 4).unwrap()[0], 255);
        req.template = "enhance/edit".into();
        assert_eq!(backend.run(&req, &NoProgress).unwrap().texts, vec!["Enhanced: the subject"]);
        let bad = FakeNative::start_with(Options { fail: Some("injected error".into()), ..Default::default() }).unwrap();
        assert!(super::backend(&bad.url).unwrap().run(&req, &NoProgress).unwrap_err().to_string().contains("injected error"));
    }
    #[test]
    fn native_timeout_cancels_the_worker_and_reports_a_timeout() {
        let fake = FakeNative::start_with(Options { delay_ms: 2000, ..Default::default() }).unwrap();
        let backend = backend(&fake.url).unwrap();
        backend.set_timeout(Duration::from_secs(1));
        let mut req = Request::new("krea2-turbo/image", "a fox");
        req.size = Some((8, 8));
        assert!(matches!(backend.run(&req, &NoProgress), Err(crate::Error::Timeout(1))));
        assert_eq!(fake.state().cancellations, 1);
        assert_eq!(backend.status().state, "ready", "a deadline leaves the worker usable");
    }
}
