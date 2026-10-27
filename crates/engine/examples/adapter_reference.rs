//! End-to-end trusted Rust adapter registration, command dispatch and undo.
//! Run: `cargo run -p photocraft-engine --example adapter_reference`
//!
//! This is a small CPU reference, NOT an AI/model runtime, GPU/NPU probe, Photoshop filter
//! loader, DLL loader or worker launcher. Its resource is opaque text and is never opened.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::sync::{Arc, Mutex, PoisonError};

use photocraft_engine::{EngineError, Session};
use photocraft_plugins::adapter::{
    self, Adapter, Context, Descriptor, Device, DeviceKind, Execution, Image, Limits, Operation, Output, OutputKind, Request, Response,
};
use serde_json::json;

const ID: &str = "org.photocraft.example.cpu-reference";
const RESOURCE: &str = "  opaque-reference://unmodified/../resource?format=original%20text  ";

#[derive(Default)]
struct CpuReference {
    last_resource: Mutex<Option<String>>,
}

impl Adapter for CpuReference {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            api_version: adapter::API_VERSION,
            id: ID.into(),
            name: "CPU Reference (not AI)".into(),
            version: "1.0.0".into(),
            operations: vec![
                Operation { id: "invert".into(), output_kind: OutputKind::Image, color_modes: vec!["rgb".into(), "gray".into()] },
                Operation { id: "first-component-mask".into(), output_kind: OutputKind::Mask, color_modes: vec!["rgb".into(), "gray".into()] },
            ],
            devices: vec![Device {
                id: "cpu.reference".into(),
                kind: DeviceKind::Cpu,
                backend: "rust-reference".into(),
                name: "Reference CPU implementation".into(),
                operations: vec!["invert".into(), "first-component-mask".into()],
            }],
            resource_formats: vec!["opaque-reference-text".into()],
            params_schema: json!({}),
        }
    }

    fn run(&self, request: &Request, ctx: &dyn Context) -> adapter::Result<Response> {
        let limits = Limits::default();
        request.image.validate(&limits)?;
        *self.last_resource.lock().unwrap_or_else(PoisonError::into_inner) = request.resource.clone();
        if ctx.cancelled() {
            return Err(adapter::Error::Canceled);
        }
        let channels = usize::from(request.image.channels);
        // Image validation checked nonzero dimensions and channel count before these loops.
        let (kind, image) = match request.operation.as_str() {
            "invert" => {
                let mut image = request.image.clone();
                let color_channels = channels.saturating_sub(usize::from(image.alpha));
                for (i, pixel) in image.data.chunks_exact_mut(channels).enumerate() {
                    if i % 256 == 0 && ctx.cancelled() {
                        return Err(adapter::Error::Canceled);
                    }
                    for sample in pixel.iter_mut().take(color_channels) {
                        *sample = 1.0 - *sample;
                    }
                }
                (OutputKind::Image, image)
            }
            "first-component-mask" => {
                // Deliberately just the first native component, not an sRGB luminance claim.
                // A production adapter defines its own color-managed image/mask semantics.
                let count = limits.check_image_size(request.image.width, request.image.height, 1)?;
                let mut data = Vec::new();
                data.try_reserve_exact(count).map_err(|_| adapter::Error::Failed("not enough memory for the reference mask".into()))?;
                for (i, pixel) in request.image.data.chunks_exact(channels).enumerate() {
                    if i % 256 == 0 && ctx.cancelled() {
                        return Err(adapter::Error::Canceled);
                    }
                    let value = pixel.first().copied().ok_or_else(|| adapter::Error::InvalidImage("missing first component".into()))?;
                    data.push(value.clamp(0.0, 1.0));
                }
                (
                    OutputKind::Mask,
                    Image {
                        x: request.image.x,
                        y: request.image.y,
                        width: request.image.width,
                        height: request.image.height,
                        channels: 1,
                        color_mode: "gray".into(),
                        source_depth: 32,
                        alpha: false,
                        icc_profile: Vec::new(),
                        data,
                    },
                )
            }
            other => return Err(adapter::Error::UnsupportedOperation(other.into())),
        };
        ctx.progress(1.0);
        Ok(Response {
            output: Output { kind, image },
            execution: Execution { devices: vec!["cpu.reference".into()], backend: "rust-reference".into(), fallback_reason: None },
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::new();
    println!("Initially: {}", session.execute("plugin.adapter.list", json!({}))?);
    let reference = Arc::new(CpuReference::default());
    session.register_adapter(reference.clone())?;
    println!("Registered: {}", session.execute("plugin.adapter.list", json!({}))?);
    session.execute("file.new", json!({"width": 4, "height": 3, "depth": 32}))?;
    session.execute("layer.new.layer", json!({}))?;
    session.edit("Reference native float pixels", |doc, active| {
        let id = active.ok_or_else(|| EngineError::Other("reference document has no active layer".into()))?;
        let area = doc.bounds();
        let surface = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| EngineError::Other("reference target has no pixels".into()))?;
        surface.try_write_region(area, &[0.25, 0.5, 0.75, 1.0].repeat(12))?;
        Ok(())
    })?;
    let before = session.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let result = session.execute(
        "plugin.adapter.run",
        json!({
            "provider": ID, "operation": "invert", "output": "pixels", "resource": RESOURCE,
            "params": {}, "device": "cpu.reference", "allowFallback": false,
        }),
    )?;
    println!("Image result: {result}");
    if reference.last_resource.lock().unwrap_or_else(PoisonError::into_inner).as_deref() != Some(RESOURCE) {
        return Err(EngineError::Other("opaque resource was not preserved".into()).into());
    }
    println!("Opaque resource arrived byte-for-byte unchanged; no resource was opened.");
    session.execute("edit.undo", json!({}))?;
    if !Arc::ptr_eq(&session.active().ok_or(EngineError::NoDocument)?.doc, &before) {
        return Err(EngineError::Other("undo did not restore the original document".into()).into());
    }
    println!("One undo restored the original image.");
    let result = session.execute(
        "plugin.adapter.run",
        json!({
            "provider": ID, "operation": "first-component-mask", "output": "selection", "resource": RESOURCE,
            "device": "cpu.reference",
        }),
    )?;
    println!("Mask-to-selection result: {result}");
    session.execute("edit.undo", json!({}))?;
    if session.active().ok_or(EngineError::NoDocument)?.doc.selection.is_some() {
        return Err(EngineError::Other("undo did not remove the generated selection".into()).into());
    }
    if !session.unregister_adapter(ID) {
        return Err(EngineError::Other("reference registration was missing".into()).into());
    }
    println!("Removed through trusted Rust API: {}", session.execute("plugin.adapter.list", json!({}))?);
    Ok(())
}
