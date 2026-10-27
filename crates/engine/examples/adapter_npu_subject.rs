//! Synthetic CPU subject-mask adapter example for PhotoCraft (not AI or NPU inference).
//! Run: `cargo run -p photocraft-engine --example adapter_npu_subject`
//!
//! The historical filename is retained for existing callers. This example draws a fixed ellipse
//! without analysing image content. It loads no model, vendor SDK, DirectML/QNN or Windows AI API,
//! and makes no claim of hardware acceleration. It demonstrates only the trusted Rust adapter
//! contract, `select.subject` dispatch, selection writeback, cancellation and single-step undo.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::sync::Arc;

use photocraft_engine::{EngineError, Session};
use photocraft_plugins::adapter::{
    self, Adapter, Context, Descriptor, Device, DeviceKind, Execution, Image, Limits, Operation, Output, OutputKind, Request, Response,
};
use serde_json::json;

const ADAPTER_ID: &str = "org.photocraft.example.synthetic-cpu-subject";
const DEVICE_ID: &str = "cpu.synthetic-ellipse";
const BACKEND: &str = "synthetic-cpu";

/// A deterministic CPU-only mock: its ellipse is unrelated to the pixels or any real subject.
struct SyntheticCpuSubjectAdapter;

impl Adapter for SyntheticCpuSubjectAdapter {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            api_version: adapter::API_VERSION,
            id: ADAPTER_ID.into(),
            name: "Synthetic CPU ellipse mask (not AI)".into(),
            version: "1.0.0".into(),
            operations: vec![Operation { id: "select-subject".into(), output_kind: OutputKind::Mask, color_modes: vec!["rgb".into()] }],
            devices: vec![Device {
                id: DEVICE_ID.into(),
                kind: DeviceKind::Cpu,
                backend: BACKEND.into(),
                name: "CPU synthetic ellipse generator".into(),
                operations: vec!["select-subject".into()],
            }],
            resource_formats: Vec::new(),
            params_schema: json!({}),
        }
    }

    fn run(&self, request: &Request, ctx: &dyn Context) -> adapter::Result<Response> {
        let limits = Limits::default();
        request.image.validate(&limits)?;
        if request.operation != "select-subject" {
            return Err(adapter::Error::UnsupportedOperation(request.operation.clone()));
        }
        if request.image.color_mode != "rgb" {
            return Err(adapter::Error::UnsupportedColorMode(request.image.color_mode.clone()));
        }
        if let Some(device) = &request.device
            && device != DEVICE_ID
        {
            return Err(adapter::Error::UnknownDevice(device.clone()));
        }
        if request.resource.is_some() || request.params.as_object().is_none_or(|params| !params.is_empty()) {
            return Err(adapter::Error::InvalidParams("the synthetic ellipse accepts no resources or adapter parameters".into()));
        }
        if ctx.cancelled() {
            return Err(adapter::Error::Canceled);
        }

        let (w, h) = (request.image.width, request.image.height);
        let count = limits.check_image_size(w, h, 1)?;
        let mut mask_data = Vec::new();
        mask_data.try_reserve_exact(count).map_err(|e| adapter::Error::Limit(e.to_string()))?;

        // Pure geometry on the CPU, not segmentation or a substitute for a deployable model.
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.5;
        let rx = w as f32 * 0.3;
        let ry = h as f32 * 0.3;
        for y in 0..h {
            if ctx.cancelled() {
                return Err(adapter::Error::Canceled);
            }
            ctx.progress(y as f32 / h as f32);
            let dy = (y as f32 - cy) / ry;
            for x in 0..w {
                let dx = (x as f32 - cx) / rx;
                let dist_sq = dx * dx + dy * dy;
                mask_data.push(if dist_sq <= 1.0 { 1.0 } else { 0.0 });
            }
        }
        if ctx.cancelled() {
            return Err(adapter::Error::Canceled);
        }
        ctx.progress(1.0);

        Ok(Response {
            output: Output {
                kind: OutputKind::Mask,
                image: Image {
                    x: request.image.x,
                    y: request.image.y,
                    width: w,
                    height: h,
                    channels: 1,
                    color_mode: "gray".into(),
                    source_depth: 32,
                    alpha: false,
                    icc_profile: Vec::new(),
                    data: mask_data,
                },
            },
            execution: Execution { devices: vec![DEVICE_ID.into()], backend: BACKEND.into(), fallback_reason: None },
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 200, "height": 160, "depth": 8}))?;

    session.register_adapter(Arc::new(SyntheticCpuSubjectAdapter))?;
    println!("Registered synthetic CPU example (not AI/NPU): {}", session.execute("plugin.adapter.list", json!({}))?);

    let res = session.execute("select.subject", json!({"provider": ADAPTER_ID, "device": DEVICE_ID}))?;
    println!("Select Subject result (fixed synthetic CPU ellipse): {res}");
    if session.active().ok_or(EngineError::NoDocument)?.doc.selection.is_none() {
        return Err(EngineError::Other("expected selection from the synthetic CPU ellipse".into()).into());
    }

    session.execute("edit.undo", json!({}))?;
    if session.active().ok_or(EngineError::NoDocument)?.doc.selection.is_some() {
        return Err(EngineError::Other("undo did not clear the selection".into()).into());
    }
    println!("Single-step undo cleared the synthetic selection.");

    if !session.unregister_adapter(ADAPTER_ID) {
        return Err(EngineError::Other("synthetic CPU adapter registration was missing".into()).into());
    }
    println!("Unregistered the synthetic CPU adapter.");
    Ok(())
}
