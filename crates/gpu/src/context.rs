//! Native windowless wgpu context creation for headless composition and export.

use std::fmt;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::health::DeviceHealth;
use crate::{Compositor, Unsupported};

/// Errors encountered when creating or inspecting a native GPU context.
#[derive(Debug, thiserror::Error)]
pub enum GpuContextError {
    #[error("no suitable GPU adapter found: {0}")]
    NoAdapter(String),
    #[error("failed to request GPU device: {0}")]
    RequestDevice(String),
    #[error("failed to initialize GPU compositor: {0}")]
    Compositor(String),
    #[error("GPU device was lost: {0}")]
    DeviceLost(String),
    /// Initialization did not finish within [`NativeContextOptions::timeout`]. Its thread is
    /// still running inside wgpu and the driver; `pending` tells when it has finished.
    #[error("no suitable GPU adapter found: timed out after {timeout:?} requesting GPU adapter/device")]
    TimedOut { timeout: Duration, pending: AbandonedRequest },
}

impl From<Unsupported> for GpuContextError {
    fn from(e: Unsupported) -> Self {
        GpuContextError::Compositor(e.0)
    }
}

/// Configuration options for creating a [`NativeContext`].
#[derive(Clone, Debug)]
pub struct NativeContextOptions {
    /// Desired adapter power tier. Default is [`wgpu::PowerPreference::HighPerformance`]
    /// to prefer discrete/dedicated hardware GPUs.
    pub power_preference: wgpu::PowerPreference,
    /// Force software/fallback adapter selection (e.g. WARP/llvmpipe).
    pub force_fallback_adapter: bool,
    /// Allowed backend flags. Default is all available native backends.
    pub backends: wgpu::Backends,
    /// Bounded timeout for adapter/device initialization. When set, requests exceeding
    /// this duration return [`GpuContextError::NoAdapter`]. Default is 10 seconds.
    ///
    /// # Limitation
    /// If initialization times out, the background initialization thread cannot be forcibly terminated
    /// in standard Rust and continues detached until wgpu completes or fails. Callers should avoid
    /// repeatedly requesting initialization in tight loops when timeouts occur to prevent thread and
    /// resource leaks. Exiting while that thread is still inside the driver can crash the process,
    /// so wait for it first: [`AbandonedRequest::wait`] on the one in
    /// [`GpuContextError::TimedOut`], or [`NativeContext::wait_for_abandoned_requests`] for all.
    /// Either returns `false` when the thread is still running at the limit; the process then
    /// either waits longer or exits accepting that risk.
    pub timeout: Option<Duration>,
}

impl Default for NativeContextOptions {
    fn default() -> Self {
        Self {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            backends: wgpu::Backends::all(),
            timeout: Some(Duration::from_secs(10)),
        }
    }
}

/// A headless, windowless GPU context managing a [`wgpu::Device`], [`wgpu::Queue`],
/// and device health monitor.
#[derive(Clone)]
pub struct NativeContext {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    adapter_info: wgpu::AdapterInfo,
    health: DeviceHealth,
    acc_format: wgpu::TextureFormat,
    limits: wgpu::Limits,
}

impl fmt::Debug for NativeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeContext")
            .field("adapter", &self.adapter_info.name)
            .field("backend", &self.adapter_info.backend)
            .field("device_type", &self.adapter_info.device_type)
            .field("acc_format", &self.acc_format)
            .field("healthy", &self.is_healthy())
            .finish()
    }
}

impl NativeContext {
    /// Create a windowless GPU context using default high-performance options.
    pub fn new() -> Result<Self, GpuContextError> {
        Self::request(&NativeContextOptions::default())
    }

    /// Request an adapter and device matching `options`.
    pub fn request(options: &NativeContextOptions) -> Result<Self, GpuContextError> {
        let opts = options.clone();
        let init_work = move || -> Result<Self, GpuContextError> {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor { backends: opts.backends, ..wgpu::InstanceDescriptor::new_without_display_handle_from_env() });

            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: opts.power_preference,
                force_fallback_adapter: opts.force_fallback_adapter,
                compatible_surface: None,
                ..Default::default()
            }))
            .or_else(|e| {
                if !opts.force_fallback_adapter {
                    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::None,
                        force_fallback_adapter: true,
                        compatible_surface: None,
                        ..Default::default()
                    }))
                } else {
                    Err(e)
                }
            })
            .map_err(|e| GpuContextError::NoAdapter(format!("request_adapter failed: {e}")))?;

            let info = adapter.get_info();
            log::info!("GPU native context: selected adapter '{}' ({:?}, {:?})", info.name, info.backend, info.device_type);

            let limits = adapter.limits();
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("pc_headless_gpu"),
                required_features: wgpu::Features::empty(),
                required_limits: limits.clone(),
                ..Default::default()
            }))
            .map_err(|e| GpuContextError::RequestDevice(format!("request_device failed for '{}': {e}", info.name)))?;

            let health = DeviceHealth::watch(&device);
            let acc_format = Compositor::preferred_acc_format(&adapter);

            Ok(Self { device: Arc::new(device), queue: Arc::new(queue), adapter_info: info, health, acc_format, limits })
        };

        if let Some(timeout) = options.timeout {
            match run_with_timeout("pc_gpu_init", timeout, init_work) {
                Ok(res) => res,
                Err(InitWait::TimedOut(pending)) => Err(GpuContextError::TimedOut { timeout, pending }),
                Err(InitWait::Disconnected) => Err(GpuContextError::NoAdapter("GPU initialization thread disconnected".into())),
                Err(InitWait::Spawn(e)) => Err(GpuContextError::NoAdapter(format!("failed to spawn GPU initialization thread: {e}"))),
            }
        } else {
            init_work()
        }
    }

    /// Wait up to `limit` for initialization threads that [`NativeContext::request`] gave up on
    /// after its timeout. Returns whether none is still running.
    ///
    /// A request that timed out leaves its thread inside wgpu and the driver. If the process
    /// exits meanwhile, the driver's state is torn down under that thread, and software drivers
    /// such as Lavapipe crash the whole process (a SIGSEGV or heap corruption after every test
    /// already passed). Call this before exiting after a timed-out request. To wait for one
    /// request only, use the [`AbandonedRequest`] in its [`GpuContextError::TimedOut`].
    ///
    /// Returns `false` when one is still running at `limit`; see [`NativeContextOptions::timeout`].
    pub fn wait_for_abandoned_requests(limit: Duration) -> bool {
        wait_until(&ABANDONED.running, &ABANDONED.done, limit, |running| *running == 0)
    }

    /// Build a new [`Compositor`] wired to this context's device and health flag.
    pub fn create_compositor(&self) -> Result<Compositor, GpuContextError> {
        if !self.is_healthy() {
            let fault = self.health.fault().map_or_else(|| "GPU device lost".into(), |f| f.to_string());
            return Err(GpuContextError::DeviceLost(fault));
        }
        let mut comp = Compositor::try_new_with_format(&self.device, self.acc_format)?;
        comp.set_health(self.health.clone());
        Ok(comp)
    }

    /// Whether this adapter is a CPU software rasterizer (e.g. llvmpipe, Lavapipe, WARP).
    pub fn is_software(&self) -> bool {
        self.adapter_info.device_type == wgpu::DeviceType::Cpu
    }

    /// Human-readable adapter name reported by the driver.
    pub fn adapter_name(&self) -> &str {
        &self.adapter_info.name
    }

    /// Backend graphics API in use (Vulkan, Metal, Dx12, etc.).
    pub fn backend(&self) -> wgpu::Backend {
        self.adapter_info.backend
    }

    /// The underlying [`wgpu::Device`].
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Arc clone of the device handle.
    pub fn device_arc(&self) -> Arc<wgpu::Device> {
        self.device.clone()
    }

    /// The underlying [`wgpu::Queue`].
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Arc clone of the queue handle.
    pub fn queue_arc(&self) -> Arc<wgpu::Queue> {
        self.queue.clone()
    }

    /// Shared device health monitor.
    pub fn health(&self) -> &DeviceHealth {
        &self.health
    }

    /// Preferred accumulation texture format for this adapter (`Rgba32Float` or `Rgba16Float`).
    pub fn acc_format(&self) -> wgpu::TextureFormat {
        self.acc_format
    }

    /// Effective device limits.
    pub fn limits(&self) -> &wgpu::Limits {
        &self.limits
    }

    /// Maximum 2D texture side dimension supported by this device.
    pub fn max_texture_dimension_2d(&self) -> u32 {
        self.limits.max_texture_dimension_2d
    }

    /// Whether the device is healthy and has not reported a lost device or uncaptured error.
    pub fn is_healthy(&self) -> bool {
        self.health.is_ok()
    }
}

/// Why [`run_with_timeout`] returned without the work's result.
enum InitWait {
    TimedOut(AbandonedRequest),
    Disconnected,
    Spawn(std::io::Error),
}

/// Run `work` on a thread named `name` and wait up to `timeout` for its result. On a timeout the
/// thread keeps running, and the returned [`AbandonedRequest`] tells when it has finished.
fn run_with_timeout<T, F>(name: &str, timeout: Duration, work: F) -> Result<T, InitWait>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    // A rendezvous channel: a result is either handed to this thread or, once it stopped
    // waiting, dropped on the worker before the worker counts as finished.
    let (tx, rx) = std::sync::mpsc::sync_channel(0);
    let request = AbandonedRequest(Arc::new(InitThread::default()));
    let thread_request = request.clone();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        // Declared first so it is dropped last, after an unclaimed result.
        let _finish = FinishOnDrop(thread_request);
        let res = work();
        let _ = tx.send(res);
    });
    if let Err(e) = spawned {
        return Err(InitWait::Spawn(e));
    }
    match rx.recv_timeout(timeout) {
        Ok(res) => Ok(res),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            request.0.abandon();
            // Stop taking the result, so the worker drops it itself.
            drop(rx);
            Err(InitWait::TimedOut(request))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(InitWait::Disconnected),
    }
}

/// Wait up to `limit` until `done` holds for the state behind `lock`, re-checking whenever `cv`
/// is notified. A limit too far ahead to represent waits as long as it takes.
fn wait_until<S>(lock: &Mutex<S>, cv: &Condvar, limit: Duration, done: impl Fn(&S) -> bool) -> bool {
    let deadline = Instant::now().checked_add(limit);
    let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
    while !done(&state) {
        state = match deadline {
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return false;
                }
                cv.wait_timeout(state, left).unwrap_or_else(PoisonError::into_inner).0
            }
            None => cv.wait(state).unwrap_or_else(PoisonError::into_inner),
        };
    }
    true
}

/// An initialization thread that [`NativeContext::request`] stopped waiting for after its
/// timeout, still running inside wgpu and the driver until it finishes.
#[derive(Clone)]
pub struct AbandonedRequest(Arc<InitThread>);

impl AbandonedRequest {
    /// Wait up to `limit` for this thread to finish. Returns `false` when it is still running;
    /// see [`NativeContextOptions::timeout`].
    pub fn wait(&self, limit: Duration) -> bool {
        wait_until(&self.0.state, &self.0.done, limit, |state| state.finished)
    }

    /// Whether the thread has finished, including dropping a context nobody claimed.
    pub fn is_finished(&self) -> bool {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner).finished
    }
}

impl fmt::Debug for AbandonedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AbandonedRequest").field("finished", &self.is_finished()).finish()
    }
}

/// How many initialization threads [`NativeContext::request`] gave up on are still running.
struct Abandoned {
    running: Mutex<usize>,
    done: Condvar,
}

static ABANDONED: Abandoned = Abandoned { running: Mutex::new(0), done: Condvar::new() };

#[derive(Default)]
struct ThreadState {
    /// The request stopped waiting for the thread.
    abandoned: bool,
    /// The thread has finished.
    finished: bool,
}

/// One initialization thread and a condition variable notified when it finishes.
#[derive(Default)]
struct InitThread {
    state: Mutex<ThreadState>,
    done: Condvar,
}

impl InitThread {
    /// The request stopped waiting: count the thread in [`ABANDONED`] unless it already finished.
    fn abandon(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.finished && !state.abandoned {
            state.abandoned = true;
            *ABANDONED.running.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        }
    }

    fn finish(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.finished = true;
        self.done.notify_all();
        if state.abandoned {
            let mut running = ABANDONED.running.lock().unwrap_or_else(PoisonError::into_inner);
            *running = running.saturating_sub(1);
            ABANDONED.done.notify_all();
        }
    }
}

/// Marks the initialization thread finished when it ends, also when it panics.
struct FinishOnDrop(AbandonedRequest);

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        self.0.0.finish();
    }
}

#[cfg(test)]
mod tests;
