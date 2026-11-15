//! Estimate Depth through the shared Craft AI layer (storytold/craft-ai-integrations).
//!
//! The model, its files, where they live and how they are set up belong to the shared layer: the
//! tool is `estimate-depth` (capability `image.depth`). This module is the app's side only: it asks
//! the layer whether the tool can run, sends a picture, and reads the 16-bit gray picture back.
//! Nothing here names a model, an engine or a file, and nothing is downloaded: setup is the shared
//! Set up AI flow (the launcher, or the app's own screen over the same pieces).

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use craft_ai_client::{AiClient, AppTools, Job, RoleStatus};
use craft_ai_core::{AppId, ConsentLedger, Event, InputRole, JobState, Locality, Output, Payload};
use craft_ai_native::shared::SharedStore;
use serde_json::json;

use super::{Quality, bad};
use crate::jobs::JobCtx;
use crate::{EngineError, Result};

/// The catalogue id of the tool.
pub const TOOL: &str = "estimate-depth";
/// This app's id in the shared layer (consent and per-app choices are scoped by it).
const APP: &str = "photocraft";
/// The longest the tool may take before it is stopped.
const LIMIT: Duration = Duration::from_secs(10 * 60);
/// How often a job on this computer is looked at (an online one once a second).
const LOCAL_STEP: Duration = Duration::from_millis(15);
const REMOTE_STEP: Duration = Duration::from_secs(1);
/// How often a waiting job looks at its cancel flag.
const CANCEL_TICK: Duration = Duration::from_millis(30);
/// How long a status answer is reused, so enabling checks do not read the shared store each frame.
const STATUS_TTL: Duration = Duration::from_secs(1);
/// Largest picture sent to the tool, in bytes (the layer's inline limit is 32 MiB).
pub(super) const MAX_SEND_BYTES: usize = 32 * 1024 * 1024 - 1;
/// Largest picture accepted back, in pixels.
const MAX_RESULT_PIXELS: u64 = 1 << 26;

/// A test hook that replaces the tool: it gets the PNG, its size and the quality name, and returns
/// a 16-bit (or 8-bit) gray PNG of the same size, white = near.
pub type Estimator = Arc<dyn Fn(&[u8], u32, u32, &str) -> std::result::Result<Vec<u8>, String> + Send + Sync>;

/// Where the shared AI store is and how a client over it is made. The app's own windows (the
/// missing-model popup, the setup screen) build their folder desk and tool desk from the same one.
#[derive(Clone)]
pub struct AiHost {
    shared: SharedStore,
    client: Arc<dyn Fn() -> std::result::Result<AiClient, String> + Send + Sync>,
}

impl AiHost {
    /// The real shared store and key stores. `CRAFT_AI_DIR` moves the store (tests and the CLI use
    /// it so they never read a person's settings).
    pub fn standard() -> std::result::Result<Self, String> {
        let shared = SharedStore::at_default_location().map_err(|e| e.to_string())?;
        let client = Arc::new(|| {
            let app = AppId::new(APP).map_err(|e| e.to_string())?;
            AiClient::standard(app).map_err(|e| e.message)
        });
        Ok(Self { shared, client })
    }

    /// A host over any store and client maker, for tests.
    pub fn new(shared: SharedStore, client: impl Fn() -> std::result::Result<AiClient, String> + Send + Sync + 'static) -> Self {
        Self { shared, client: Arc::new(client) }
    }

    /// The shared store.
    pub fn shared(&self) -> &SharedStore {
        &self.shared
    }

    /// A new client for this app.
    pub fn client(&self) -> std::result::Result<AiClient, String> {
        (self.client)()
    }
}

/// What the session keeps of the AI layer. Shared behind an `Arc` so the background status refresh
/// can write its answer; the locks let a `&Session` (the `enabled` checks) create things lazily.
#[derive(Default)]
pub struct AiState {
    /// Replaces the tool when set (tests).
    pub estimator: Option<Estimator>,
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    host: Mutex<Option<AiHost>>,
    /// Why the standard host could not be made (no home folder), kept so the commands say it
    /// instead of the generic "not set up in this frontend".
    host_error: Mutex<Option<String>>,
    tools: Mutex<Option<Arc<AppTools>>>,
    status: Mutex<Option<(web_time::Instant, Option<String>)>>,
    /// Bumped when the cached status is forgotten, so a refresh that began before is not stored.
    epoch: AtomicU64,
    /// A background refresh is running (at most one at a time).
    refreshing: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Said while the first status answer is being read in the background.
const CHECKING: &str = "Checking Estimate Depth\u{2026}";

impl Inner {
    fn host(&self) -> std::result::Result<AiHost, String> {
        lock(&self.host).clone().ok_or_else(|| lock(&self.host_error).clone().unwrap_or_else(|| "Estimate Depth is not set up in this frontend".to_owned()))
    }

    fn tools(&self) -> std::result::Result<Arc<AppTools>, String> {
        let mut slot = lock(&self.tools);
        if let Some(tools) = slot.as_ref() {
            return Ok(tools.clone());
        }
        let tools = Arc::new(AppTools::with_client(self.host()?.client()?));
        *slot = Some(tools.clone());
        Ok(tools)
    }

    /// Reads the shared store, the key store and the model files: slow, never on the UI thread
    /// per frame.
    fn compute(&self) -> Option<String> {
        match self.tools() {
            Err(e) => Some(e),
            Ok(tools) => match tools.status(TOOL) {
                RoleStatus::Ready(_) => None,
                RoleStatus::Unavailable { reason } => Some(reason),
            },
        }
    }

    /// Computes the status and keeps it, unless it was forgotten meanwhile.
    fn refresh(&self) -> Option<String> {
        let epoch = self.epoch.load(Ordering::SeqCst);
        let reason = self.compute();
        let mut slot = lock(&self.status);
        if self.epoch.load(Ordering::SeqCst) == epoch {
            *slot = Some((web_time::Instant::now(), reason.clone()));
        }
        reason
    }
}

impl AiState {
    /// The host the frontend injected. A session without one (tests, embedders) has no AI layer:
    /// nothing reads a person's AI settings unless a frontend asks for it with
    /// [`use_standard_host`](Self::use_standard_host).
    pub fn host(&self) -> std::result::Result<AiHost, String> {
        self.inner.host()
    }

    /// Uses the person's real shared AI store (`CRAFT_AI_DIR` moves it). The desktop app and the
    /// CLI call this; the error is the sentence to show (no home folder).
    pub fn use_standard_host(&self) -> std::result::Result<(), String> {
        match AiHost::standard() {
            Ok(host) => {
                self.set_host(host);
                Ok(())
            }
            Err(reason) => {
                self.fail_host(reason.clone());
                Err(reason)
            }
        }
    }

    /// Keeps `reason` as why there is no host, so [`host`](Self::host) and the status say it.
    fn fail_host(&self, reason: String) {
        *lock(&self.inner.host_error) = Some(reason);
        self.forget_status();
    }

    /// The same AI layer for another session: the host, the tools and the status are this one's
    /// (a scratch session of Batch or a droplet runs actions that estimate depth), and so is the
    /// test hook. A session without a host gives one without a host, so tests stay isolated.
    pub fn shared(&self) -> AiState {
        AiState { estimator: self.estimator.clone(), inner: self.inner.clone() }
    }

    /// Uses `host` instead of the real store (tests). Forgets the tools and the cached status.
    pub fn set_host(&self, host: AiHost) {
        *lock(&self.inner.host) = Some(host);
        *lock(&self.inner.host_error) = None;
        *lock(&self.inner.tools) = None;
        self.forget_status();
    }

    /// The tools view over the host, made on first use. A failure is not kept, so a later call
    /// tries again.
    pub fn tools(&self) -> std::result::Result<Arc<AppTools>, String> {
        self.inner.tools()
    }

    /// Why Estimate Depth cannot run now, in words for the person, or `None` when it can. Never
    /// blocks: it returns the last answer at once and, when that is older than a second, reads the
    /// store again on a background thread. Before the first answer it says the tool is being
    /// checked. For a decision that must be right (running the command), use
    /// [`status_reason_now`](Self::status_reason_now).
    pub fn status_reason(&self) -> Option<String> {
        if self.estimator.is_some() {
            return None;
        }
        let known = lock(&self.inner.status).clone();
        match known {
            Some((at, reason)) => {
                if web_time::Instant::now().duration_since(at) >= STATUS_TTL {
                    self.refresh_in_background();
                }
                reason
            }
            None => {
                self.refresh_in_background();
                Some(CHECKING.to_owned())
            }
        }
    }

    /// The answer for the command's `enabled` check. Once an answer is known it is returned at
    /// once (and refreshed in the background when older than a second). The first time, none is
    /// known, so it is read now, on this thread, and kept: reading `enabled` twice must give one
    /// answer (a listing of the commands read twice must not change), which a provisional "enabled"
    /// while a background read runs would not.
    pub fn known_status_reason(&self) -> Option<String> {
        if self.estimator.is_some() {
            return None;
        }
        let known = lock(&self.inner.status).clone();
        match known {
            Some((at, reason)) => {
                if web_time::Instant::now().duration_since(at) >= STATUS_TTL {
                    self.refresh_in_background();
                }
                reason
            }
            None => self.inner.refresh(),
        }
    }

    /// As [`status_reason`](Self::status_reason) but reads the store now, on this thread, and keeps
    /// the answer. For a person's action, not for a frame.
    pub fn status_reason_now(&self) -> Option<String> {
        if self.estimator.is_some() {
            return None;
        }
        self.inner.refresh()
    }

    /// Starts one background refresh unless one is already running.
    fn refresh_in_background(&self) {
        if self.inner.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let inner = self.inner.clone();
        let started = std::thread::Builder::new().name("photocraft-depth-status".into()).spawn(move || {
            // A panic in the shared layer must not leave `refreshing` set for good.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.refresh()));
            inner.refreshing.store(false, Ordering::SeqCst);
        });
        if started.is_err() {
            self.inner.refreshing.store(false, Ordering::SeqCst);
        }
    }

    /// Drops the cached status; the next [`status_reason`](Self::status_reason) reads it again.
    pub fn forget_status(&self) {
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        *lock(&self.inner.status) = None;
    }

    /// What a job calls: send `png` (`w` x `h`) and get the gray PNG back.
    pub(super) fn backend(&self) -> std::result::Result<Backend, String> {
        if let Some(e) = self.estimator.clone() {
            return Ok(Arc::new(move |_, png, w, h, q| e(&png, w, h, q.name()).map_err(bad)));
        }
        let tools = self.tools()?;
        Ok(Arc::new(move |ctx, png, _, _, q| call_tool(&tools, ctx, png, q)))
    }
}

/// The estimate step of a job: PNG in, gray PNG out.
pub(super) type Backend = Arc<dyn Fn(&JobCtx, Vec<u8>, u32, u32, Quality) -> Result<Vec<u8>> + Send + Sync>;

/// Run the tool: request, plan, run (waiting off to one side so the job's cancel reaches it).
fn call_tool(tools: &AppTools, ctx: &JobCtx, png: Vec<u8>, quality: Quality) -> Result<Vec<u8>> {
    let request = tools.request(TOOL).ok_or_else(|| bad("Estimate Depth is not available here"))?;
    let source = Payload::inline("image/png", png).map_err(|e| bad(e.message))?;
    let request = request.with_input(InputRole::Source, source).with_param("quality", json!(quality.name()));
    let plan = tools.plan(TOOL, request).map_err(bad)?;
    // A local model needs no consent. If the person's choice would send the picture away, this
    // command does not ask for them: that prompt belongs to the app's windows.
    let ledger = ConsentLedger::default();
    if plan.needs_consent(&ledger) {
        return Err(bad("The chosen model for Estimate Depth needs your agreement first. Use Estimate Depth from the brush settings in the app to review it."));
    }
    // This job's own handle: cancelling it must not stop another document's estimate, which the
    // session's shared tools may be waiting for at the same time.
    let job = tools.client().start(&plan, &ledger).map_err(|e| bad(e.message))?;
    let step = match plan.disclosure().locality {
        Locality::Remote => REMOTE_STEP,
        Locality::Local => LOCAL_STEP,
    };
    let done = wait_cancellable(|| ctx.cancelled(), || job.cancel().is_ok(), || job.wait(step, LIMIT))
        .unwrap_or_else(|| Err(craft_ai_core::AiError::invalid("Estimate Depth stopped unexpectedly")));
    if ctx.cancelled() {
        return Err(EngineError::Cancelled);
    }
    // A wait that ended without the job ending asks it to stop; say so when it would not.
    let done =
        done.map_err(|e| if stopped(&job) { bad(e.message) } else { bad(format!("{} It could not be stopped, so it may still be running.", e.message)) })?;
    match done.state {
        JobState::Succeeded => {}
        JobState::Failed { error } => return Err(bad(error.message)),
        JobState::Cancelled => return Err(EngineError::Cancelled),
        _ => return Err(bad("The job did not finish.")),
    }
    done.events
        .into_iter()
        .find_map(|e| match e {
            Event::Output(Output { payload: Payload::Inline { media_type, data }, .. }) if media_type.as_str() == "image/png" => Some(data),
            _ => None,
        })
        .ok_or_else(|| bad("Estimate Depth returned no picture"))
}

/// True when `job` stopped or had already ended after its wait gave up.
fn stopped(job: &Job) -> bool {
    match job.cancel() {
        Ok(()) => true,
        Err(_) => job.poll().is_ok_and(|snap| snap.state.is_terminal()),
    }
}

/// Runs `wait` on a worker thread and, while it runs, calls `cancel` every tick once `cancelled`
/// says so (asked again each tick: the job may not have been known to the service the first time).
/// `cancel` belongs to the one job `wait` waits for, never to a shared list of jobs. `None` when
/// the worker panicked.
fn wait_cancellable<T: Send>(cancelled: impl Fn() -> bool + Sync, cancel: impl Fn() -> bool + Sync, wait: impl FnOnce() -> T + Send) -> Option<T> {
    std::thread::scope(|scope| {
        let worker = scope.spawn(wait);
        while !worker.is_finished() {
            if cancelled() {
                let _ = cancel();
            }
            std::thread::sleep(CANCEL_TICK);
        }
        worker.join().ok()
    })
}

/// Encode interleaved 8-bit RGB as a PNG (size-checked against the layer's inline limit).
pub(super) fn encode_rgb8(rgb: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().map_err(|e| bad(format!("could not encode the picture: {e}")))?;
        writer.write_image_data(rgb).map_err(|e| bad(format!("could not encode the picture: {e}")))?;
    }
    if out.len() > MAX_SEND_BYTES {
        return Err(bad("the picture is too large to send for depth estimation"));
    }
    Ok(out)
}

/// A decoded depth map: row-major values in 0..1 (larger = nearer).
pub(super) struct Map {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

/// Decode the tool's gray PNG (16- or 8-bit) into 0..1. The size must be the one that was sent.
pub(super) fn decode_gray(bytes: &[u8], w: u32, h: u32) -> Result<Map> {
    let invalid = || bad("Estimate Depth returned a picture that could not be read");
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().map_err(|_| invalid())?;
    let info = reader.info();
    let (iw, ih, color, depth) = (info.width, info.height, info.color_type, info.bit_depth);
    if (iw, ih) != (w, h) {
        return Err(bad(format!("Estimate Depth returned a {iw} x {ih} picture for a {w} x {h} one")));
    }
    if u64::from(iw) * u64::from(ih) > MAX_RESULT_PIXELS {
        return Err(bad("Estimate Depth returned a picture that is too large"));
    }
    let wide = match (color, depth) {
        (png::ColorType::Grayscale, png::BitDepth::Sixteen) => true,
        (png::ColorType::Grayscale, png::BitDepth::Eight) => false,
        _ => return Err(bad("Estimate Depth returned a picture that is not 8- or 16-bit gray")),
    };
    let size = reader.output_buffer_size().ok_or_else(invalid)?;
    let expected = (iw as usize).saturating_mul(ih as usize).saturating_mul(if wide { 2 } else { 1 });
    if size != expected {
        return Err(invalid());
    }
    let mut buf = vec![0u8; size];
    reader.next_frame(&mut buf).map_err(|_| invalid())?;
    let data = if wide {
        buf.as_chunks::<2>().0.iter().map(|p| f32::from(u16::from_be_bytes(*p)) / 65535.0).collect()
    } else {
        buf.iter().map(|&v| f32::from(v) / 255.0).collect()
    };
    Ok(Map { width: iw, height: ih, data })
}

#[cfg(test)]
pub(crate) fn encode_gray16(data: &[u16], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Sixteen);
    let mut writer = enc.write_header().unwrap();
    let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_be_bytes()).collect();
    writer.write_image_data(&bytes).unwrap();
    drop(writer);
    out
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    /// A job as the wait sees it: it ends when released or when asked to stop, and counts the stops.
    struct Fake {
        release: AtomicBool,
        stop: AtomicBool,
        stops: AtomicUsize,
    }

    impl Fake {
        fn new() -> Self {
            Self { release: AtomicBool::new(false), stop: AtomicBool::new(false), stops: AtomicUsize::new(0) }
        }

        fn wait(&self) -> &'static str {
            loop {
                if self.stop.load(Ordering::SeqCst) {
                    return "stopped";
                }
                if self.release.load(Ordering::SeqCst) {
                    return "done";
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn cancel(&self) -> bool {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.stop.store(true, Ordering::SeqCst);
            true
        }
    }

    /// Cancelling one document's estimate stops only that one while another keeps running.
    #[test]
    fn cancelling_one_wait_leaves_a_concurrent_one_running() {
        let (a, b) = (Fake::new(), Fake::new());
        let (cancel_a, cancel_b) = (AtomicBool::new(false), AtomicBool::new(false));
        std::thread::scope(|scope| {
            let first = scope.spawn(|| wait_cancellable(|| cancel_a.load(Ordering::SeqCst), || a.cancel(), || a.wait()));
            let second = scope.spawn(|| wait_cancellable(|| cancel_b.load(Ordering::SeqCst), || b.cancel(), || b.wait()));
            std::thread::sleep(Duration::from_millis(100));
            cancel_a.store(true, Ordering::SeqCst);
            assert_eq!(first.join().unwrap(), Some("stopped"));
            // The other job was never asked to stop and still runs.
            std::thread::sleep(Duration::from_millis(150));
            assert!(!second.is_finished());
            assert_eq!(b.stops.load(Ordering::SeqCst), 0);
            b.release.store(true, Ordering::SeqCst);
            assert_eq!(second.join().unwrap(), Some("done"));
        });
        assert!(a.stops.load(Ordering::SeqCst) >= 1);
        assert_eq!(b.stops.load(Ordering::SeqCst), 0);
    }

    /// The reason the standard host could not be made is what the commands say, until a host is set.
    #[test]
    fn a_failed_standard_host_keeps_its_reason() {
        let state = AiState::default();
        assert_eq!(state.host().err().as_deref(), Some("Estimate Depth is not set up in this frontend"));
        state.fail_host("no home folder to keep AI settings in".to_owned());
        assert_eq!(state.host().err().as_deref(), Some("no home folder to keep AI settings in"));
        assert_eq!(state.status_reason_now().as_deref(), Some("no home folder to keep AI settings in"));
        // A host that works replaces the failure.
        let host = AiHost::new(SharedStore::new("/nonexistent/x.json"), || Err("later".to_owned()));
        state.set_host(host);
        assert!(state.host().is_ok());
        assert_eq!(state.status_reason_now().as_deref(), Some("later"));
    }

    /// A shared state sees the parent's host and status; one made from a state without a host has none.
    #[test]
    fn a_shared_state_uses_the_parents_host() {
        let parent = AiState::default();
        assert!(parent.shared().host().is_err());
        parent.set_host(AiHost::new(SharedStore::new("/nonexistent/x.json"), || Err("parent host".to_owned())));
        let child = parent.shared();
        assert!(child.host().is_ok());
        assert_eq!(child.status_reason_now().as_deref(), Some("parent host"));
    }

    #[test]
    fn a_panicking_wait_is_none() {
        let done: Option<u8> = wait_cancellable(|| false, || true, || -> u8 { std::panic::panic_any("boom") });
        assert_eq!(done, None);
    }
}
