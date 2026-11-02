//! Native windowless wgpu context creation for headless composition and export.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

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
    /// Optional timeout for adapter/device initialization.
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
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor { backends: options.backends, ..wgpu::InstanceDescriptor::new_without_display_handle_from_env() });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: options.power_preference,
            force_fallback_adapter: options.force_fallback_adapter,
            compatible_surface: None,
        }))
        .map_err(|e| GpuContextError::NoAdapter(format!("request_adapter failed: {e}")))?;

        let info = adapter.get_info();
        log::info!("GPU native context: selected adapter '{}' ({:?}, {:?})", info.name, info.backend, info.device_type);

        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pc_headless_gpu"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            memory_hints: Default::default(),
        }))
        .map_err(|e| GpuContextError::RequestDevice(format!("request_device failed for '{}': {e}", info.name)))?;

        let health = DeviceHealth::watch(&device);
        let acc_format = Compositor::preferred_acc_format(&adapter);

        Ok(Self { device: Arc::new(device), queue: Arc::new(queue), adapter_info: info, health, acc_format, limits })
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
        self.health.is_healthy()
    }
}
