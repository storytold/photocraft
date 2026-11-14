//! Native windowless wgpu context creation for headless composition and export.

use std::fmt;
use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::health::DeviceHealth;
use crate::{Compositor, Unsupported};

/// Whether an initialization thread has finished, shared with the request that started it.
type InitState = Arc<(Mutex<bool>, Condvar)>;

/// Marks an initialization thread finished when it returns or unwinds.
struct InitDone(InitState);

impl Drop for InitDone {
    fn drop(&mut self) {
        let (finished, changed) = &*self.0;
        *finished.lock().unwrap_or_else(PoisonError::into_inner) = true;
        changed.notify_all();
    }
}

/// An initialization thread that was still running when its request timed out.
///
/// A process that exits while the thread is still inside the graphics driver can crash on exit.
/// Call [`AbandonedInit::wait`] before the process exits.
#[derive(Clone)]
pub struct AbandonedInit {
    state: InitState,
    thread: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Test hook: told each time [`AbandonedInit::wait`] starts waiting for unfinished work.
    waiting: Option<mpsc::Sender<()>>,
}

impl fmt::Debug for AbandonedInit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AbandonedInit").field("finished", &self.is_finished()).finish()
    }
}

impl AbandonedInit {
    /// Run `work` on a thread named `name` and track that thread.
    fn start(name: &str, work: impl FnOnce() + Send + 'static) -> std::io::Result<Self> {
        let state: InitState = Arc::new((Mutex::new(false), Condvar::new()));
        let done = InitDone(state.clone());
        let handle = std::thread::Builder::new().name(name.into()).spawn(move || {
            let _done = done;
            work();
        })?;
        Ok(Self { state, thread: Arc::new(Mutex::new(Some(handle))), waiting: None })
    }

    /// Test hook: run `work` on its own thread and track it as an initialization thread that
    /// outlived its timeout, so code that waits for one can be tested with a thread the test holds.
    /// `work` receives a message each time [`AbandonedInit::wait`] starts waiting for it; the
    /// receiver disconnects once every clone of the returned value is dropped.
    #[doc(hidden)]
    pub fn spawn(work: impl FnOnce(mpsc::Receiver<()>) + Send + 'static) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let mut init = Self::start("pc_gpu_init_test", move || work(rx))?;
        init.waiting = Some(tx);
        Ok(init)
    }

    /// Whether the thread has exited, after dropping any late result.
    pub fn is_finished(&self) -> bool {
        let mut thread = self.thread.lock().unwrap_or_else(PoisonError::into_inner);
        match thread.take() {
            None => true,
            Some(handle) if handle.is_finished() => {
                let _ = handle.join();
                true
            }
            Some(handle) => {
                *thread = Some(handle);
                false
            }
        }
    }

    /// Wait up to `limit` for the thread's work to finish, then join the thread. Returns whether
    /// the work finished. The join also waits for the thread's thread-local destructors, which
    /// `limit` does not cover.
    pub fn wait(&self, limit: Duration) -> bool {
        self.wait_from(Instant::now(), limit)
    }

    /// [`AbandonedInit::wait`] with its limit counted from `start`.
    fn wait_from(&self, start: Instant, limit: Duration) -> bool {
        let (finished, changed) = &*self.state;
        let mut done = finished.lock().unwrap_or_else(PoisonError::into_inner);
        let mut told = false;
        while !*done {
            let Some(left) = limit.checked_sub(start.elapsed()).filter(|left| !left.is_zero()) else {
                return false;
            };
            // Tell the test hook only once some time is left, so a wait that has already run out
            // does not release the thread.
            if !told {
                told = true;
                if let Some(waiting) = &self.waiting {
                    let _ = waiting.send(());
                }
            }
            done = changed.wait_timeout(done, left).unwrap_or_else(PoisonError::into_inner).0;
        }
        drop(done);
        // The thread has left the driver; join it so its thread-local destructors have run too.
        // The lock stays held until the join returns, so `is_finished` on a clone waits for it.
        let mut thread = self.thread.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(handle) = thread.take() {
            let _ = handle.join();
        }
        drop(thread);
        true
    }
}

/// Run `work` on its own thread and wait up to `timeout` for its result. When the wait times out,
/// the error comes back with an [`AbandonedInit`] for the thread, which drops the late result itself.
fn run_with_timeout<T: Send + 'static>(
    name: &str,
    timeout: Duration,
    work: impl FnOnce() -> Result<T, GpuContextError> + Send + 'static,
) -> (Result<T, GpuContextError>, Option<AbandonedInit>) {
    // With no buffer, a send completes only when the receiver takes the value, so a late result is
    // never queued for the requesting thread to drop.
    let (tx, rx) = std::sync::mpsc::sync_channel(0);
    let thread_res = AbandonedInit::start(name, move || {
        let res = work();
        // A late result is dropped here, before the thread is marked finished.
        let _ = tx.send(res);
        drop(tx);
    });
    let init = match thread_res {
        Ok(init) => init,
        Err(e) => return (Err(GpuContextError::NoAdapter(format!("failed to spawn GPU initialization thread: {e}"))), None),
    };
    match rx.recv_timeout(timeout) {
        Ok(res) => (res, None),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            let err = GpuContextError::NoAdapter(format!("timed out after {timeout:?} requesting GPU adapter/device"));
            (Err(err), Some(init))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => (Err(GpuContextError::NoAdapter("GPU initialization thread disconnected".into())), None),
    }
}

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
    /// in standard Rust and continues until wgpu completes or fails. [`NativeContext::request_tracked`]
    /// returns an [`AbandonedInit`] for such a thread. Callers should avoid repeatedly requesting
    /// initialization in tight loops when timeouts occur to prevent thread and resource leaks.
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
        Self::request_tracked(options).0
    }

    /// Like [`NativeContext::request`], and also returns the initialization thread when the
    /// request timed out while it was still running.
    pub fn request_tracked(options: &NativeContextOptions) -> (Result<Self, GpuContextError>, Option<AbandonedInit>) {
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

        match options.timeout {
            Some(timeout) => run_with_timeout("pc_gpu_init", timeout, init_work),
            None => (init_work(), None),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Sets its flag when it is dropped on the initialization thread.
    struct Late(Arc<AtomicBool>);

    impl Drop for Late {
        fn drop(&mut self) {
            if std::thread::current().name() == Some("pc_gpu_init_test") {
                self.0.store(true, Ordering::SeqCst);
            }
        }
    }

    #[test]
    fn a_timed_out_request_returns_its_thread_and_the_thread_drops_the_late_result() {
        let (release, released) = std::sync::mpsc::channel::<()>();
        let dropped = Arc::new(AtomicBool::new(false));
        let late = Late(dropped.clone());
        let (res, abandoned) = run_with_timeout("pc_gpu_init_test", Duration::ZERO, move || {
            let _ = released.recv();
            Ok(late)
        });
        assert!(matches!(res, Err(GpuContextError::NoAdapter(ref msg)) if msg.contains("timed out")), "{:?}", res.err());
        let abandoned = abandoned.expect("a timed-out request returns its thread");
        assert!(!abandoned.is_finished());
        assert!(!abandoned.wait(Duration::ZERO));

        release.send(()).unwrap();
        assert!(abandoned.wait(Duration::MAX));
        assert!(abandoned.is_finished());
        assert!(dropped.load(Ordering::SeqCst), "the thread drops the late result before it is finished");
    }

    #[test]
    fn a_wait_with_no_time_left_does_not_release_the_thread() {
        let (told_tx, told) = std::sync::mpsc::channel();
        let init = AbandonedInit::spawn(move |waiting| {
            let _ = told_tx.send(waiting.recv().is_ok());
        })
        .unwrap();
        assert!(!init.wait(Duration::ZERO));
        // Dropping the last handle disconnects the thread's receiver.
        drop(init);
        assert_eq!(told.recv().ok(), Some(false), "a wait with no time left does not tell the thread it started waiting");
    }

    #[test]
    fn a_wait_whose_limit_ran_out_before_it_started_does_not_release_the_thread() {
        let (told_tx, told) = std::sync::mpsc::channel();
        let init = AbandonedInit::spawn(move |waiting| {
            let _ = told_tx.send(waiting.recv().is_ok());
        })
        .unwrap();
        // A nonzero limit that ran out before the wait took the lock, as when taking it was slow.
        let start = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        assert!(!init.wait_from(start, Duration::from_millis(500)));
        drop(init);
        assert_eq!(told.recv().ok(), Some(false), "a wait whose limit ran out does not tell the thread it started waiting");
    }

    #[test]
    fn a_request_that_finishes_in_time_returns_no_thread() {
        let (res, abandoned) = run_with_timeout("pc_gpu_init_test", Duration::MAX, || Ok(7));
        assert_eq!(res.ok(), Some(7));
        assert!(abandoned.is_none());
    }
}
