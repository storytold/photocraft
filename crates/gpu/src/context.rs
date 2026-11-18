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
    /// resource leaks. Before the process exits, call [`NativeContext::wait_for_abandoned_requests`]:
    /// exiting while that thread is still inside the driver can crash the process.
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
            let (tx, rx) = std::sync::mpsc::channel();
            let request = Arc::new(InitThread::default());
            let thread_request = Arc::clone(&request);
            let thread_res = std::thread::Builder::new().name("pc_gpu_init".into()).spawn(move || {
                // Declared first so it is dropped last: an abandoned request counts as running
                // until the context nobody waits for any more has been dropped too.
                let _finish = FinishOnDrop(thread_request);
                let res = init_work();
                let _ = tx.send(res);
            });
            match thread_res {
                Ok(_handle) => match rx.recv_timeout(timeout) {
                    Ok(res) => res,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        request.abandon();
                        Err(GpuContextError::NoAdapter(format!("timed out after {timeout:?} requesting GPU adapter/device")))
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(GpuContextError::NoAdapter("GPU initialization thread disconnected".into())),
                },
                Err(e) => Err(GpuContextError::NoAdapter(format!("failed to spawn GPU initialization thread: {e}"))),
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
    /// already passed). Call this before exiting after a timed-out request.
    pub fn wait_for_abandoned_requests(limit: Duration) -> bool {
        // A limit too far ahead to represent waits as long as it takes.
        let deadline = Instant::now().checked_add(limit);
        let mut running = ABANDONED.running.lock().unwrap_or_else(PoisonError::into_inner);
        while *running > 0 {
            running = match deadline {
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return false;
                    }
                    ABANDONED.done.wait_timeout(running, left).unwrap_or_else(PoisonError::into_inner).0
                }
                None => ABANDONED.done.wait(running).unwrap_or_else(PoisonError::into_inner),
            };
        }
        true
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

/// How many initialization threads [`NativeContext::request`] gave up on are still running.
struct Abandoned {
    running: Mutex<usize>,
    done: Condvar,
}

static ABANDONED: Abandoned = Abandoned { running: Mutex::new(0), done: Condvar::new() };

/// One initialization thread: whether its request gave up on it, and whether it has finished.
#[derive(Default)]
struct InitThread {
    state: Mutex<(bool, bool)>,
}

impl InitThread {
    /// The request stopped waiting: count the thread in [`ABANDONED`] unless it already finished.
    fn abandon(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (abandoned, finished) = &mut *state;
        if !*finished && !*abandoned {
            *abandoned = true;
            *ABANDONED.running.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        }
    }

    fn finish(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (abandoned, finished) = &mut *state;
        *finished = true;
        if *abandoned {
            let mut running = ABANDONED.running.lock().unwrap_or_else(PoisonError::into_inner);
            *running = running.saturating_sub(1);
            ABANDONED.done.notify_all();
        }
    }
}

/// Marks the initialization thread finished when it ends, also when it panics.
struct FinishOnDrop(Arc<InitThread>);

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        self.0.finish();
    }
}
