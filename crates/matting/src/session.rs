//! ONNX Runtime session management: provider selection with an explicit,
//! user-visible fallback chain (DirectML → CPU), model discovery, and a
//! process-wide session cache so a 1 GB model loads once per PhotoCraft run.
//!
//! The design follows the crate-level contract in `lib.rs`: never a silent
//! fallback — the returned [`CutoutResult`] always says which provider served
//! the request and on which adapter, and failure of *both* providers surfaces
//! as [`MattingError::NoProvider`] with both error strings.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Instant,
};

use crate::{
    device::{self, GpuInfo},
    error::{MattingError, Result},
    postprocess::build_alpha,
    preprocess::build_input,
};

/// Which BiRefNet export to load. Every export takes a fixed square
/// input; the variants differ in resolution (edge sharpness vs compute).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelVariant {
    /// BiRefNet-general at 1024×1024 — the default and the fast one.
    #[default]
    General,
    /// BiRefNet-HR at 2048×2048 — sharper detail at ~4× the compute.
    Hr,
}

impl ModelVariant {
    pub fn resolution(self) -> u32 {
        match self {
            Self::General => crate::DEFAULT_RESOLUTION,
            Self::Hr => 2048,
        }
    }

    fn file_names(self) -> &'static [&'static str] {
        match self {
            // File names probed under every model root; weights are
            // user-supplied (never shipped) — see the crate docs.
            Self::General => &["birefnet-general.onnx"],
            Self::Hr => &["birefnet-hr.onnx", "birefnet_hr.onnx"],
        }
    }
}

/// Device preference for the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DevicePref {
    /// DirectML on the best adapter when one exists, else CPU.
    #[default]
    Auto,
    /// Force the CPU provider (also the escape hatch when a GPU misbehaves).
    Cpu,
    /// Force the DirectML device id (the index from [`crate::enumerate_gpus`]).
    Gpu(u32),
}

/// Inference options; defaults follow the crate docs.
#[derive(Debug, Clone)]
pub struct CutoutOptions {
    pub variant: ModelVariant,
    pub device: DevicePref,
}

impl Default for CutoutOptions {
    fn default() -> Self {
        Self { variant: ModelVariant::default(), device: DevicePref::default() }
    }
}

/// What one [`cutout`] call reports back.
#[derive(Debug, Clone)]
pub struct CutoutResult {
    /// `width * height` alpha values, 255 = keep.
    pub alpha: Vec<u8>,
    /// `"DirectML"` or `"CPU"` — which provider actually served the run.
    pub provider: String,
    /// The adapter used, when a GPU served the run.
    pub gpu: Option<GpuInfo>,
    /// Wall-clock milliseconds of preprocess + inference + postprocess.
    pub ms: u128,
}

/// Where the model file is searched: next to the executable (portable), the
/// data dir a deploy script creates, then the user cache.
pub fn model_candidates(variant: ModelVariant) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(d) = exe_dir {
        roots.push(d.join("PhotoCraftData").join("models").join("matting"));
        roots.push(d.join("models").join("matting"));
    }
    if let Ok(c) = std::env::var("LOCALAPPDATA") {
        roots.push(Path::new(&c).join("PhotoCraft").join("models").join("matting"));
    }
    if let Ok(extra) = std::env::var("PHOTOCRAFT_MODEL_DIR") {
        roots.push(PathBuf::from(extra).join("matting"));
    }
    for name in variant.file_names() {
        for root in &roots {
            out.push(root.join(name));
        }
    }
    out
}

fn find_model(variant: ModelVariant) -> Result<PathBuf> {
    let searched = model_candidates(variant);
    for p in &searched {
        if p.is_file() {
            return Ok(p.clone());
        }
    }
    Err(MattingError::ModelNotFound { searched: searched.iter().map(|p| p.display().to_string()).collect() })
}

// ---- session cache -------------------------------------------------------

struct Cached {
    key: (PathBuf, u32, u8),
    session: ort::session::Session,
    provider: String,
    gpu: Option<GpuInfo>,
}

static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();

/// Release the cached session (call on application shutdown so ONNX Runtime's
/// teardown happens before process teardown, never on a lazy static drop).
pub fn release() {
    if let Some(c) = CACHE.get() {
        if let Ok(mut guard) = c.lock() {
            *guard = None;
        }
    }
}

/// Human-readable runtime report for the command's status output.
pub fn ort_status() -> String {
    format!("ONNX Runtime {}", ort::info())
}

fn cache() -> &'static Mutex<Option<Cached>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

fn device_tag(d: DevicePref) -> u8 {
    match d {
        DevicePref::Auto => 0,
        DevicePref::Cpu => 1,
        DevicePref::Gpu(id) => 2 + id as u8,
    }
}

/// Open (or reuse) a session for `variant` honouring `device`, returning the
/// live session, which provider served it, and the adapter when a GPU did.
/// The caller holds the cache lock; this only builds.
fn session_for(path: &Path, variant: ModelVariant, device: DevicePref) -> Result<(ort::session::Session, String, Option<GpuInfo>)> {
    let _ = variant;
    build_session(path, variant, device)
}

/// The process-wide ONNX Runtime environment (created lazily on first use).
/// With `load-dynamic` this loads `onnxruntime.dll` from the executable's
/// directory; failure is remembered so we never retry-load every call.
fn env() -> Result<ort::environment::Environment> {
    static ENV: std::sync::OnceLock<Option<ort::environment::Environment>> = std::sync::OnceLock::new();
    // The plugin registration handle must outlive every session: dropping it
    // unloads the EP factory and its devices vanish from Environment::devices.
    static OV_LIB: std::sync::OnceLock<Option<ort::ep::ExecutionProviderLibrary>> = std::sync::OnceLock::new();
    let slot = ENV.get_or_init(|| {
        let env = ort::init().build().ok();
        let handle = env.as_ref().and_then(|e| {
            let mut candidates = Vec::new();
            if let Ok(d) = std::env::var("PHOTOCRAFT_OV_PLUGIN_DIR") {
                for ep in ort_plugin_names() {
                    candidates.push(std::path::PathBuf::from(&d).join(ort_plugin_name(ep)));
                }
            }
            let mut push = |dir: &std::path::Path, v: &mut Vec<std::path::PathBuf>| {
                for ep in ort_plugin_names() {
                    v.push(dir.join(ort_plugin_name(ep)));
                }
                v.push(dir.join("models").join("matting").join(ort_plugin_name("openvino_plugin")));
            };
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    push(dir, &mut candidates);
                }
            }
            if let Ok(cwd) = std::env::current_dir() {
                push(&cwd, &mut candidates);
            }
            candidates.into_iter().find(|p| p.exists()).and_then(|lib| match e.register_ep_library("openvino", &lib) {
                Ok(h) => Some(h),
                Err(err) => {
                    log::warn!("[matting] OpenVINO EP library at {} failed to register: {err}", lib.display());
                    None
                }
            })
        });
        OV_LIB.set(handle).ok();
        env
    });
    slot.clone().ok_or(MattingError::OrtLibraryMissing)
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn ort_plugin_names() -> [&'static str; 2] {
    // wheel-built official package name first, legacy fork name second
    ["openvino_plugin", "openvino"]
}

fn ort_plugin_name(ep: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        format!("onnxruntime_providers_{ep}.dll").into()
    } else if cfg!(target_os = "macos") {
        format!("libonnxruntime_providers_{ep}.dylib").into()
    } else {
        format!("libonnxruntime_providers_{ep}.so").into()
    }
}

fn build_session(path: &Path, _variant: ModelVariant, device: DevicePref) -> Result<(ort::session::Session, String, Option<GpuInfo>)> {
    log::debug!("[matting] building a session (device {device:?})");
    let gpu: Option<GpuInfo> = match device {
        DevicePref::Cpu => None,
        DevicePref::Auto => device::pick_gpu(),
        DevicePref::Gpu(id) => device::enumerate_gpus().into_iter().find(|g| g.index == id),
    };

    if device != DevicePref::Cpu {
        if let Some(g) = &gpu {
            // OpenVINO first on Intel GPUs: it is Intel's own Arc path and
            // computes BiRefNet correctly where DirectML's EP collapses (see
            // the logits guard in cutout). The OV plugin DLLs are optional —
            // without them registration/commit fails here and we fall through
            // to DirectML, so machines without the plugins behave exactly as
            // before.
            if g.vendor_id == 0x8086 {
                let cache_dir = path.parent().map(|d| d.join("ov-cache").display().to_string()).unwrap_or_default();
                // V2 device selection: the plugin EP ignores every device-ish
                // provider option ("pass the enumerated EP device to
                // AppendExecutionProvider_V2"), so enumerate the registered
                // OpenVINO devices and bind the GPU one explicitly. Everything
                // else travels inside load_config: the EP's legacy option
                // names (cache_dir, precision, ...) are advise-only and
                // otherwise silently ignored — OV plugin properties go in the
                // device-keyed JSON instead (CACHE_DIR alongside
                // INFERENCE_PRECISION_HINT). FP32 matters: the GPU plugin's
                // default fp16 oneDNN kernels crash executing BiRefNet on Arc
                // (primitive-exec error); FP32 runs correctly at ~26x CPU
                // speed on an A770, and with the compile cache in the same
                // JSON the kernel build is actually persisted across runs.
                let mut attempt = |cache: bool| -> Result<ort::session::Session> {
                    let load_config = if cache {
                        format!(r#"{{"GPU": {{"INFERENCE_PRECISION_HINT": "f32", "CACHE_DIR": {}}}}}"#, json_string(&cache_dir))
                    } else {
                        r#"{"GPU": {"INFERENCE_PRECISION_HINT": "f32"}}"#.to_owned()
                    };
                    let opts = vec![("OpenVINOExecutionProvider.load_config".to_owned(), load_config)];
                    let env = env()?;
                    let gpu_devices = env
                        .devices()
                        .filter(|d| d.ep().unwrap_or("") == "OpenVINOExecutionProvider" && d.hardware_device().ty() == ort::memory::DeviceType::GPU)
                        .collect::<Vec<_>>();
                    if gpu_devices.is_empty() {
                        return Err(MattingError::Session("no OpenVINO GPU device registered".into()));
                    }
                    let mut b = ort::session::Session::builder(&env).map_err(|e| MattingError::Session(e.to_string()))?;
                    b.with_devices(gpu_devices, Some(&opts))
                        .map_err(|e| MattingError::Session(e.to_string()))?
                        .commit_from_file(path)
                        .map_err(|e| MattingError::Session(e.to_string()))
                };
                match attempt(true) {
                    Ok(s) => {
                        log::debug!("[matting] OpenVINO GPU session (fp32) on {}", g.describe());
                        return Ok((s, "OpenVINO".to_owned(), Some(g.clone())));
                    }
                    Err(e1) => {
                        // A poisoned or incompatible cache blob must not take
                        // the GPU path down with it: retry uncached.
                        let e1s = e1.to_string();
                        match attempt(false) {
                            Ok(s) => {
                                log::warn!("[matting] OpenVINO GPU session (fp32, cache failed: {e1s}) on {}", g.describe());
                                return Ok((s, "OpenVINO".to_owned(), Some(g.clone())));
                            }
                            Err(e2) => log::debug!("[matting] OpenVINO GPU unavailable ({e1s}; uncached retry: {e2}); trying DirectML"),
                        }
                    }
                }
            }
            log::debug!("[matting] DirectML candidate: {}", g.describe());
            let attempt = ort::session::Session::builder(&env()?)
                .map_err(|e| MattingError::Session(e.to_string()))?
                .with_execution_providers([ort::ep::DirectML::default().with_device_id(g.index as i32).build()])
                .map_err(|e| MattingError::Session(e.to_string()))?
                .commit_from_file(path)
                .map_err(|e| MattingError::Session(e.to_string()));
            match attempt {
                Ok(s) => return Ok((s, "DirectML".to_owned(), Some(g.clone()))),
                Err(dml_err) => {
                    // DirectML failed (missing DLL, device id, driver) — the
                    // CPU fallback is honest only if we say why we fell back.
                    log::warn!("DirectML session failed ({dml_err}); falling back to CPU");
                    let cpu_err = match cpu_session(path) {
                        Ok(s) => return Ok((s, "CPU".to_owned(), None)),
                        Err(e) => e,
                    };
                    return Err(MattingError::NoProvider { detail: format!("DirectML: {dml_err}; CPU: {cpu_err}") });
                }
            }
        }
    }
    match cpu_session(path) {
        Ok(s) => Ok((s, "CPU".to_owned(), None)),
        Err(e) => Err(match device {
            DevicePref::Auto if gpu.is_none() => e,
            _ => e,
        }),
    }
}

fn cpu_session(path: &Path) -> std::result::Result<ort::session::Session, MattingError> {
    log::debug!("[matting] building the CPU session");
    ort::session::Session::builder(&env()?)
        .map_err(|e| MattingError::Session(e.to_string()))?
        .with_execution_providers([ort::ep::CPU::default().build()])
        .map_err(|e| MattingError::Session(e.to_string()))?
        .commit_from_file(path)
        .map_err(|e| MattingError::Session(e.to_string()))
}

// ---- the one entry point ---------------------------------------------------

/// Run the matting model over straight RGBA8 pixels and return the alpha matte plus
/// the provenance of the run (provider, adapter, timings).
///
/// A GPU session is serialised through the cache lock: one 200 M-parameter
/// model per process is enough, and ONNX Runtime sessions are thread-safe for
/// `run` — the lock protects the *cache*, letting concurrent callers share the
/// session they find there.
pub fn cutout(rgba: &[u8], width: u32, height: u32, opts: &CutoutOptions) -> Result<CutoutResult> {
    let start = Instant::now();
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return Err(MattingError::BadBuffer { expected, got: rgba.len(), width, height, channels: 4 });
    }
    let path = find_model(opts.variant)?;
    let (input, padding) = build_input(rgba, width, height, opts.variant.resolution());

    let mut guard = cache().lock().map_err(|_| MattingError::Session("cache poisoned".to_owned()))?;
    let key = (path.clone(), opts.variant.resolution(), device_tag(opts.device));
    let cache_hit = guard.as_ref().is_some_and(|c| c.key == key);
    if !cache_hit {
        let (session, provider, gpu) = session_for(&path, opts.variant, opts.device)?;
        *guard = Some(Cached { key: key.clone(), session, provider: provider.clone(), gpu: gpu.clone() });
    }
    let cached = guard.as_mut().ok_or_else(|| MattingError::Session("cache empty after fill".to_owned()))?;

    let (shape, mut logits) = {
        use ort::value::Tensor;
        let side = opts.variant.resolution() as usize;
        let tensor = Tensor::from_array(([1usize, 3, side, side], input.clone().into_boxed_slice())).map_err(|e| MattingError::Inference(e.to_string()))?;
        let outputs = cached.session.run(ort::inputs![tensor]).map_err(|e| MattingError::Inference(e.to_string()))?;
        if outputs.len() == 0 {
            return Err(MattingError::Inference("model returned no outputs".to_owned()));
        }
        let ref out = outputs[0];
        let (_shape, data) = out.try_extract_tensor::<f32>().map_err(|e| MattingError::Inference(e.to_string()))?;
        // Copy: the output buffer lives inside the Value we drop next.
        let owned = data.to_vec();
        (owned.len(), owned)
    };
    let side = opts.variant.resolution() as usize;
    if shape != side * side {
        // Some exports emit 1×S×S, others 1×1×S×S; the plane is the same.
        return Err(MattingError::Inference(format!("unexpected output plane: {shape} samples, expected {}", side * side)));
    }
    let max = logits.iter().copied().fold(f32::MIN, f32::max);
    let mut provider = cached.provider.clone();
    let mut gpu = cached.gpu.clone();
    if provider != "CPU" && max <= 0.0 {
        // A GPU EP can collapse BiRefNet-family logits on some drivers: every
        // output lands below 0, so the matte would be uniformly transparent
        // (seen with DirectML on Intel Arc). That is indistinguishable from a
        // background-only frame, but a real frame with a subject has positive
        // logits somewhere — rather than silently shipping an empty matte,
        // redo the run on CPU, where the collapse never happens, and cache
        // that path so the wasted GPU attempt does not repeat.
        log::warn!("[matting] {provider} inference collapsed (max logit {max:.2}); re-running on CPU");
        use ort::value::Tensor;
        let mut cpu = cpu_session(&path)?;
        let tensor = Tensor::from_array(([1usize, 3, side, side], input.into_boxed_slice())).map_err(|e| MattingError::Inference(e.to_string()))?;
        let owned = {
            let outputs = cpu.run(ort::inputs![tensor]).map_err(|e| MattingError::Inference(e.to_string()))?;
            let ref out = outputs[0];
            let (_shape, data) = out.try_extract_tensor::<f32>().map_err(|e| MattingError::Inference(e.to_string()))?;
            data.to_vec()
        };
        if owned.len() != side * side {
            return Err(MattingError::Inference(format!("unexpected output plane: {} samples, expected {}", owned.len(), side * side)));
        }
        logits = owned;
        provider = "CPU".to_owned();
        gpu = None;
        *guard = Some(Cached { key: (path, opts.variant.resolution(), device_tag(DevicePref::Cpu)), session: cpu, provider: "CPU".to_owned(), gpu: None });
    }
    let alpha = build_alpha(&logits, &padding, width, height);
    Ok(CutoutResult { alpha, provider, gpu, ms: start.elapsed().as_millis() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_search_is_deterministic_and_named() {
        let c = model_candidates(ModelVariant::General);
        assert!(c.iter().any(|p| p.file_name().is_some_and(|f| f == "birefnet-general.onnx")));
        assert!(model_candidates(ModelVariant::Hr).iter().any(|p| p.file_name().is_some_and(|f| f == "birefnet-hr.onnx")));
        assert_eq!(c, model_candidates(ModelVariant::General));
    }

    #[test]
    fn bad_buffer_is_a_value_not_a_panic() {
        let e = cutout(&[0u8; 3], 2, 2, &CutoutOptions::default()).unwrap_err();
        assert!(matches!(e, MattingError::BadBuffer { .. }), "{e}");
    }
}
