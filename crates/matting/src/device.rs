//! GPU detection (native, all platforms).
//!
//! The adapter list answers two questions for the caller:
//!
//! 1. *What is on this machine?* — [`enumerate_gpus`] returns every adapter
//!    with its vendor, device type and the wgpu backend that saw it.
//! 2. *Which one should DirectML use?* — [`pick_gpu`] ranks hardware
//!    adapters (discrete before integrated, D3D12/DirectML-reachable first)
//!    and returns the winner; its index in the D3D12 adapter list is the
//!    DirectML device id.
//!
//! Vendor ids follow the PCI registry: 0x10DE NVIDIA, 0x1002 AMD,
//! 0x8086 Intel, 0x1414 Microsoft (software). An Intel-only laptop shows
//! exactly one hardware adapter — "识别显卡" is not a formality, it is what
//! makes the DirectML device id meaningful on multi-GPU machines.
//!
//! The enumeration is safe Rust on top of wgpu (already in the dependency
//! tree); no DXGI interop is written here.

use serde::Serialize;
use wgpu::{Backends, DeviceType};

/// PCI vendor id of the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Microsoft,
    Other(u32),
}

impl Vendor {
    pub fn from_id(id: u32) -> Self {
        match id {
            0x10DE => Self::Nvidia,
            0x1002 => Self::Amd,
            0x8086 => Self::Intel,
            0x1414 => Self::Microsoft,
            other => Self::Other(other),
        }
    }

    /// Short lowercase tag used in reports (`"nvidia"`, `"amd"`, `"intel"`…).
    pub fn tag(&self) -> String {
        match self {
            Self::Nvidia => "nvidia".to_owned(),
            Self::Amd => "amd".to_owned(),
            Self::Intel => "intel".to_owned(),
            Self::Microsoft => "software".to_owned(),
            Self::Other(id) => format!("vendor-{id:#06x}"),
        }
    }
}

/// One GPU as seen by wgpu.
#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    /// Position in the backend-filtered enumeration; the DirectML device id.
    pub index: u32,
    pub name: String,
    pub vendor: Vendor,
    pub vendor_id: u32,
    /// wgpu's classification: discrete / integrated / CPU (software) / other.
    pub device_type: String,
    /// The backend that enumerated this adapter (`"Dx12"`, `"Vulkan"`…).
    pub backend: String,
    /// True for software adapters (llvmpipe / WARP / Microsoft Basic Render).
    pub software: bool,
}

impl GpuInfo {
    /// e.g. `"Intel(R) Arc(TM) A770 Graphics (intel, discrete, Dx12, device 0)"`.
    pub fn describe(&self) -> String {
        format!("{} ({}, {}, {}, device {})", self.name, self.vendor.tag(), self.device_type, self.backend, self.index)
    }

    /// Ranking key for [`pick_gpu`]: discrete first, then integrated, then the
    /// rest; ties broken by the backend that found it (D3D12 first, so the
    /// index lines up with what DirectML will use).
    fn rank(&self) -> (u8, u8) {
        let type_rank = match self.device_type.as_str() {
            "DiscreteGpu" => 0,
            "VirtualGpu" => 1,
            "IntegratedGpu" => 2,
            "Other" => 3,
            _ => 4,
        };
        let backend_rank = match self.backend.as_str() {
            "Dx12" => 0,
            "Vulkan" => 1,
            "Metal" => 1,
            "Gl" => 2,
            _ => 3,
        };
        (type_rank, backend_rank)
    }
}

/// Every hardware adapter, D3D12 ones first (their order matches DXGI, which
/// is what the DirectML device id refers to).
pub fn enumerate_gpus() -> Vec<GpuInfo> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: Backends::DX12 | Backends::VULKAN | Backends::GL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapters = pollster::block_on(instance.enumerate_adapters(Backends::all()));
    let mut out: Vec<GpuInfo> = adapters
        .into_iter()
        .filter_map(|a| {
            let info = a.get_info();
            let software = matches!(info.device_type, DeviceType::Cpu);
            Some(GpuInfo {
                index: 0, // assigned after sorting, below
                name: info.name,
                vendor: Vendor::from_id(info.vendor),
                vendor_id: info.vendor,
                device_type: format!("{:?}", info.device_type),
                backend: format!("{:?}", info.backend),
                software,
            })
        })
        .collect();
    // Software rasterizers never accelerate ONNX inference; keep them out.
    out.retain(|g| !g.software);
    // D3D12-reachable adapters first: DirectML device ids are DXGI indices.
    out.sort_by_key(|g| g.rank());
    for (i, g) in out.iter_mut().enumerate() {
        g.index = i as u32;
    }
    out
}

/// The GPU the matting session should use, if the machine has one.
pub fn pick_gpu() -> Option<GpuInfo> {
    enumerate_gpus().into_iter().next()
}

/// Human-readable report for logs and the command result.
#[allow(dead_code)] // consumed by the engine's status reporting once wired.
pub fn report() -> String {
    let gpus = enumerate_gpus();
    if gpus.is_empty() {
        return "no hardware GPU detected (wgpu enumeration)".to_owned();
    }
    gpus.iter().map(|g| g.describe()).collect::<Vec<_>>().join("; ")
}
