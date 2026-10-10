//! Dialog-to-command adapter. The engine owns analysis; this only samples the existing proxy.
use super::*;

pub(super) fn settings(app: &mut PhotocraftApp, current: &CameraRaw) -> Result<CameraRaw, String> {
    let d = app.camera_raw.as_ref().ok_or("Camera Raw isn't open")?;
    let step = d.proxy.len().div_ceil(photocraft_algo::camera_raw::auto::MAX_SAMPLES).max(1);
    let samples: Vec<[f32; 4]> = d
        .proxy
        .iter()
        .enumerate()
        .step_by(step)
        .filter_map(|(i, pixel)| {
            if !pixel.iter().all(|v| v.is_finite()) {
                return None;
            }
            let mut pixel = *pixel;
            if let Some(coverage) = &d.selection {
                *pixel.last_mut()? *= coverage.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            }
            Some(pixel)
        })
        .collect();
    let result = app.run(
        photocraft_engine::camera_raw_auto_cmds::AUTO,
        json!({
            "samples": samples, "temperature": current.temperature, "tint": current.tint,
        }),
    )?;
    let mut next = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let fields = result.get("settings").and_then(Value::as_object).ok_or("Auto returned no settings")?;
    for (key, value) in fields {
        let target = next.get_mut(key).ok_or("Auto returned an unknown setting")?;
        *target = value.clone();
    }
    serde_json::from_value(next).map_err(|e| e.to_string())
}
