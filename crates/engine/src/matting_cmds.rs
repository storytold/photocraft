//! `layer.removeBackgroundAI` — BiRefNet neural matting as an engine command.
//!
//! Reads the active pixel layer, runs the network through
//! [`photocraft_matting::cutout`] (DirectML GPU when available, CPU
//! otherwise — the result always reports which served the run), and multiplies
//! the returned matte into the layer's alpha in a single undoable edit. The
//! command never deletes pixels: users who want a hole rather than a matte
//! flatten afterwards, and a mask round-trip stays lossless.

use serde_json::{Value, json};

use photocraft_geom::Rect;

use crate::{EngineError, Result, Session};

const CMD: &str = "layer.removeBackgroundAI";

/// Run the model over the active layer and matte out its background.
///
/// Params: `{"variant":"general"|"hr"="general","device":"auto"|"cpu"=auto}`.
/// Returns `{"provider":"DirectML"|"CPU","gpu":name|null,"ms":elapsed}`.
pub fn remove_background_ai(s: &mut Session, p: &Value) -> Result<Value> {
    let variant = match p.get("variant").and_then(Value::as_str).unwrap_or("general") {
        "general" => photocraft_matting::ModelVariant::General,
        "hr" => photocraft_matting::ModelVariant::Hr,
        other => {
            return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("`variant` must be \"general\" or \"hr\", got {other:?}") });
        }
    };
    let device = match p.get("device").and_then(Value::as_str).unwrap_or("auto") {
        "auto" => photocraft_matting::DevicePref::Auto,
        "cpu" => photocraft_matting::DevicePref::Cpu,
        other => {
            return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("`device` must be \"auto\" or \"cpu\", got {other:?}") });
        }
    };

    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = st.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    let layer = st.doc.layer(id).ok_or(EngineError::NoDocument)?;
    let surface = layer.surface().ok_or_else(|| EngineError::Other(format!("{CMD} needs a pixel layer")))?;
    let area = surface.content_bounds();
    let (w, h) = (area.width(), area.height());
    if w == 0 || h == 0 {
        return Err(EngineError::Other("the active layer is empty".into()));
    }

    // Straight f32 RGBA → u8 RGBA for the model (alpha ignored; the matte is
    // what we compute). Clamping to [0,1] matches how the compositor reads.
    let mut f32s = vec![[0.0f32; 4]; (w as usize) * (h as usize)];
    surface.read_rgba_into(Rect::new(area.x0, area.y0, area.x1, area.y1), &mut f32s);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let rgba: Vec<u8> = f32s.iter().flat_map(|px| [q(px[0]), q(px[1]), q(px[2]), q(px[3])]).collect();

    let opts = photocraft_matting::CutoutOptions { variant, device };
    let run = photocraft_matting::cutout(&rgba, w, h, &opts).map_err(|e| EngineError::Other(format!("{CMD}: {e}")))?;

    // Multiply the matte into the layer alpha; one undo step.
    let alphas = run.alpha.clone();
    let ox = area.x0;
    let oy = area.y0;
    s.edit("Remove Background (AI)", |doc, _| {
        let surf = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| EngineError::Other("the active layer changed; nothing was applied".into()))?;
        for (i, a) in alphas.iter().enumerate() {
            let (x, y) = (ox + (i as u32 % w) as i32, oy + (i as u32 / w) as i32);
            let old = surf.rgba(x, y);
            let row = [old[0], old[1], old[2], old[3] * (*a as f32 / 255.0)];
            surf.write_pixel(x, y, &row);
        }
        Ok(())
    })?;

    Ok(json!({
        "provider": run.provider,
        "gpu": run.gpu.as_ref().map(|g| g.describe()),
        "ms": run.ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reports_bad_params_without_a_document() {
        // Params are validated before the document is touched.
        let mut s = Session::new();
        let e = remove_background_ai(&mut s, &json!({"variant": "nope"})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{e:?}");
        let e = remove_background_ai(&mut s, &json!({"device": "nope"})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{e:?}");
        // With no document the failure is NoDocument, not a panic.
        let e = remove_background_ai(&mut s, &json!({})).unwrap_err();
        assert!(matches!(e, EngineError::NoDocument), "{e:?}");
    }
}
