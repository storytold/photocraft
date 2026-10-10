//! Read-only Camera Raw parameter estimation, shared by UI and headless callers.
use photocraft_algo::camera_raw::{CameraRaw, auto};
use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session, commands::CommandSpec};

// A query namespace keeps this out of the pixel-filter mask/channel routing path.
pub const AUTO: &str = "cameraRaw.autoSettings";

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: AUTO.into(), msg: message.into() }
}

fn estimate(s: &mut Session, p: &Value) -> Result<Value> {
    let fields = p.as_object().ok_or_else(|| bad("expected an object"))?;
    for key in fields.keys() {
        if !matches!(key.as_str(), "samples" | "layer" | "temperature" | "tint") {
            return Err(bad(format!("unknown parameter {key}")));
        }
    }
    let number = |key| -> Result<f32> {
        match p.get(key) {
            None => Ok(0.0),
            Some(v) => v.as_f64().filter(|n| n.is_finite() && n.abs() <= 100.0).map(|n| n as f32).ok_or_else(|| bad(format!("{key} must be in -100..100"))),
        }
    };
    let current = CameraRaw { temperature: number("temperature")?, tint: number("tint")?, ..Default::default() };
    let samples = if let Some(values) = p.get("samples") {
        if p.get("layer").is_some() {
            return Err(bad("provide samples or a layer, not both"));
        }
        let values =
            values.as_array().filter(|v| !v.is_empty() && v.len() <= auto::MAX_SAMPLES).ok_or_else(|| bad("samples must contain 1–16384 RGBA pixels"))?;
        let mut samples = Vec::with_capacity(values.len());
        for value in values {
            let components = value.as_array().filter(|v| v.len() == 4).ok_or_else(|| bad("each sample must contain four finite numbers"))?;
            let mut rgba = [0.0; 4];
            for (out, value) in rgba.iter_mut().zip(components) {
                *out = value.as_f64().filter(|v| v.is_finite() && v.abs() <= f64::from(f32::MAX)).ok_or_else(|| bad("sample is not a finite f32"))? as f32;
            }
            samples.push(rgba);
        }
        samples
    } else {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let layer = match p.get("layer") {
            Some(v) => LayerId(v.as_u64().ok_or_else(|| bad("layer must be a layer id"))?),
            None => st.active_layer.ok_or_else(|| bad("no active layer"))?,
        };
        let surf = st.doc.layer(layer).and_then(photocraft_doc::Layer::surface).ok_or_else(|| bad("Auto needs a pixel layer or explicit preview samples"))?;
        let area = st.doc.bounds().intersect(&surf.content_bounds());
        if area.is_empty() {
            return Err(bad("no visible pixels"));
        }
        let width = i64::from(area.x1) - i64::from(area.x0);
        let height = i64::from(area.y1) - i64::from(area.y0);
        let (nx, ny) = (width.min(128), height.min(128));
        let mut samples = Vec::new();
        for y in 0..ny {
            for x in 0..nx {
                let sx = i64::from(area.x0) + (2 * x + 1) * width / (2 * nx);
                let sy = i64::from(area.y0) + (2 * y + 1) * height / (2 * ny);
                let (sx, sy) = (sx as i32, sy as i32);
                let mut rgba = surf.rgba(sx, sy);
                if let Some(mask) = &st.doc.selection
                    && let Some(alpha) = rgba.last_mut()
                {
                    *alpha *= mask.sample_channel(sx, sy, 0).clamp(0.0, 1.0);
                }
                samples.push(rgba);
            }
        }
        samples
    };
    let settings = auto::estimate(&samples, &current).map_err(bad)?;
    Ok(json!({"settings": settings}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: AUTO,
        label: "Camera Raw Auto Settings",
        menu: &[],
        shortcut: None,
        params: r#"{"layer":id? (default active pixel layer),"samples":[[r,g,b,a]]? (instead of layer; 1–16384 straight RGBA pixels; alpha includes coverage),"temperature":-100..100=0,"tint":-100..100=0} → {settings:{exposure,contrast,highlights,shadows,whites,blacks,vibrance,saturation}}; query only, white balance unchanged"#,
        enabled: crate::commands::always,
        run: estimate,
        journal: false,
    }]
}
