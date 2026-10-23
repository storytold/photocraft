//! `layer.removeBackgroundAI` — BiRefNet neural matting as an engine command.
//!
//! Reads the active pixel layer, runs the network through
//! [`photocraft_matting::cutout_with_progress`] (OpenVINO/DirectML GPU when
//! available, CPU otherwise — the result always reports which served the run),
//! and multiplies the returned matte into the layer's alpha in a single
//! undoable edit. The command never deletes pixels: users who want a hole
//! rather than a matte flatten afterwards, and a mask round-trip stays
//! lossless.
//!
//! The heavy work runs as a background job (`crate::jobs`), like the classic
//! Remove Background: the UI stays responsive, the status bar shows each
//! stage's progress (`Loading the model …`, `Running inference …`), Esc
//! cancels, and other edits to the document are locked until the run ends.
//! Cancellation lands at stage boundaries: ONNX Runtime offers no interrupt
//! inside a session build or a forward pass, so Esc waits for the current
//! stage (≤ ~1 minute on a cold GPU kernel build, ~2 s per inference) and
//! then discards the result — the document is never touched by a partial run.

use serde_json::{Value, json};

use photocraft_geom::Rect;

use crate::{EngineError, Result, Session};

const CMD: &str = "layer.removeBackgroundAI";
const CMD_HD: &str = "layer.removeBackgroundAIHd";

/// Turn a matting error into the engine error the job should report. Cancel
/// keeps its identity so the jobs framework records `cancelled`, not `failed`.
fn matting_err(cmd: &str, e: photocraft_matting::MattingError) -> EngineError {
    match e {
        photocraft_matting::MattingError::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(format!("{cmd}: {e}")),
    }
}

/// True when the 2048-px export (`birefnet-hr.onnx`) is installed; drives the
/// HD menu item's presence. Filesystem probing, so callers cache the answer.
/// On wasm there is no filesystem model store to probe, and matting itself is
/// compiled out, so the answer is a flat "not available".
pub fn hr_available() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        photocraft_matting::model_candidates(photocraft_matting::ModelVariant::Hr).into_iter().any(|p| p.exists())
    }
}

/// Run the model over the active layer and matte out its background.
///
/// Params: `{"variant":"general"|"hr"="general","device":"auto"|"cpu"="auto"}`.
/// Returns `{"provider":"OpenVINO"|"DirectML"|"CPU","gpu":name|null,"ms":elapsed}`.
pub fn remove_background_ai(s: &mut Session, p: &Value) -> Result<Value> {
    let variant = match p.get("variant") {
        Some(Value::Null) | None => photocraft_matting::ModelVariant::General,
        Some(Value::String(s)) => match s.as_str() {
            "general" => photocraft_matting::ModelVariant::General,
            "hr" => photocraft_matting::ModelVariant::Hr,
            other => {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("`variant` must be \"general\" or \"hr\", got {other:?}") });
            }
        },
        Some(other) => {
            return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("`variant` must be a string, got {other}") });
        }
    };
    let device = match p.get("device") {
        Some(Value::Null) | None => photocraft_matting::DevicePref::Auto,
        Some(Value::String(s)) => match s.as_str() {
            "auto" => photocraft_matting::DevicePref::Auto,
            "cpu" => photocraft_matting::DevicePref::Cpu,
            other => {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("`device` must be \"auto\" or \"cpu\", got {other:?}") });
            }
        },
        Some(other) => {
            return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("`device` must be a string, got {other}") });
        }
    };
    // Resolve the target synchronously so "no document / no active layer"
    // errors answer immediately; the pixels themselves are read on the
    // worker from the document snapshot.
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = st.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;

    // The HD command shares this implementation: the job label and the
    // error prefix follow the variant so history and messages name the
    // command the user actually invoked.
    let hr = matches!(variant, photocraft_matting::ModelVariant::Hr);
    let (cmdname, job_label): (&str, &str) = if hr { (CMD_HD, "Remove Background (AI, HD)") } else { (CMD, "Remove Background (AI)") };

    // Photoshop converts a Background layer to a normal layer before erasing to
    // transparency, and refuses one whose pixels are locked; mirror both.
    // The conversion and the lock check run inside the job (on the snapshot that
    // gets committed) so a converted Background is committed in the same undo step.
    crate::jobs::edit_job(
        s,
        job_label,
        move |doc, _, ctx| {
            ctx.progress(0.0, "Reading layer");
            ctx.check()?;
            crate::extra_cmds::background_to_layer(doc, id);
            crate::commands::check_pixels_unlocked(doc, id).map_err(|e| EngineError::Other(format!("{cmdname}: {e}")))?;
            if doc.effective_locks(id).transparency {
                let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
                return Err(EngineError::Other(format!(
                    "Could not complete your request because the transparency of layer \"{}\" is locked",
                    layer.name
                )));
            }
            let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let surface = layer.surface().ok_or_else(|| EngineError::Other(format!("{cmdname} needs a pixel layer")))?;
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
            drop(f32s);

            // Each stage names itself in the progress line and gates on
            // cancellation; the fractions keep the bar moving across the
            // uneven wall-clock of the stages (LoadingModel dwarfs the rest
            // when the GPU has to compile freshly).
            let opts = photocraft_matting::CutoutOptions { variant, device };
            let mut stage = |st: photocraft_matting::CutoutStage| -> std::result::Result<(), ()> {
                if ctx.cancelled() {
                    return Err(());
                }
                let (frac, msg) = match st {
                    photocraft_matting::CutoutStage::PreparingInput => (0.05, "Preparing image"),
                    photocraft_matting::CutoutStage::CheckingModel => (0.10, "Looking for the model"),
                    photocraft_matting::CutoutStage::LoadingModel => (
                        0.15,
                        if hr {
                            "Loading the model — the first GPU run compiles kernels and can take several minutes at 2048 px"
                        } else {
                            "Loading the model — the first GPU run compiles kernels and can take about a minute"
                        },
                    ),
                    photocraft_matting::CutoutStage::Inference => (0.55, "Running inference"),
                    photocraft_matting::CutoutStage::RetryingOnCpu => (0.60, "GPU returned no subject; re-running on the CPU"),
                    photocraft_matting::CutoutStage::Postprocessing => (0.90, "Building the alpha matte"),
                };
                ctx.progress(frac, msg);
                Ok(())
            };
            let run = photocraft_matting::cutout_with_progress(&rgba, w, h, &opts, &mut stage).map_err(|e| matting_err(cmdname, e))?;
            ctx.check()?;

            // Multiply the matte into the layer alpha (on this snapshot; the
            // framework commits it as one undo step on the UI thread).
            let alphas = run.alpha;
            let (ox, oy) = (area.x0, area.y0);
            let surf =
                doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| EngineError::Other("the active layer changed; nothing was applied".into()))?;
            for (i, a) in alphas.iter().enumerate() {
                let (x, y) = (ox + (i as u32 % w) as i32, oy + (i as u32 / w) as i32);
                let old = surf.rgba(x, y);
                let row = [old[0], old[1], old[2], old[3] * (*a as f32 / 255.0)];
                surf.write_pixel(x, y, &row);
            }

            Ok((run.provider, run.gpu.map(|g| g.describe()), run.ms))
        },
        move |(provider, gpu, ms)| json!({ "provider": provider, "gpu": gpu, "ms": ms }),
    )
}

/// `layer.removeBackgroundAIHd` — the same job forced onto the high-resolution
/// export. A distinct command id so the Layers menu can offer it (and hide it)
/// based on the weights actually installed. Params: `{"device":"auto|cpu"}`.
pub fn remove_background_ai_hd(s: &mut Session, p: &Value) -> Result<Value> {
    let mut p = p.clone();
    p["variant"] = json!("hr");
    remove_background_ai(s, &p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hd_command_shares_the_job_path() {
        // The HD wrapper forces the hr variant before the shared implementation
        // runs: bad device still validates, and with no document the failure
        // is NoDocument rather than a panic.
        let mut s = Session::new();
        let e = remove_background_ai_hd(&mut s, &json!({"device": "nope"})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{e:?}");
        let e = remove_background_ai_hd(&mut s, &json!({})).unwrap_err();
        assert!(matches!(e, EngineError::NoDocument), "{e:?}");
    }

    #[test]
    fn reports_bad_params_without_a_document() {
        // Params are validated before the document is touched.
        let mut s = Session::new();
        let e = remove_background_ai(&mut s, &json!({"variant": "nope"})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{e:?}");
        let e = remove_background_ai(&mut s, &json!({"device": "nope"})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{e:?}");
        // A present-but-wrong-type value is a params error, not a silent default.
        let e = remove_background_ai(&mut s, &json!({"variant": 42})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{e:?}");
        let e = remove_background_ai(&mut s, &json!({"device": false})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{e:?}");
        // An explicit JSON null behaves like an omitted field (the default).
        let e = remove_background_ai(&mut s, &json!({"variant": null, "device": null})).unwrap_err();
        assert!(matches!(e, EngineError::NoDocument), "{e:?}");
        // With no document the failure is NoDocument, not a panic.
        let e = remove_background_ai(&mut s, &json!({})).unwrap_err();
        assert!(matches!(e, EngineError::NoDocument), "{e:?}");
    }

    /// The command refuses a layer whose transparency is locked (it multiplies
    /// the matte into the alpha channel, which transparency locks protect).
    #[test]
    fn refuses_locked_transparency() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
        let e = s.execute(CMD, json!({})).unwrap_err();
        assert!(e.to_string().contains("transparency") && e.to_string().contains("locked"), "refuse transparency lock: {e}");
    }

    /// The command refuses a layer whose pixels are locked (Photoshop's message).
    /// (The Background-conversion branch is exercised at the engine level by
    /// `extra_cmds::background_to_layer`; with no model weights in the test env
    /// the job cannot run to a successful commit, so here we assert the refusal,
    /// which is synchronous even when the job would otherwise be spawned.)
    #[test]
    fn refuses_locked_pixels() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        // `file.new` starts with a single Background layer (transparency locked,
        // pixels unlocked) — a normal layer edit proceeds, so the only way the
        // command refuses here is the explicit pixels lock.
        s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
        let e = s.execute(CMD, json!({})).unwrap_err();
        assert!(e.to_string().contains("locked"), "refuse locked pixels: {e}");
    }
    /// waiting, the document is locked against edits meanwhile, and cancel
    /// ends the job with the document untouched. `"hr"` keeps this fast even
    /// where model weights exist: only the general export ships locally.
    #[test]
    fn runs_as_a_background_job_and_locks_the_document() {
        use crate::jobs::Started;
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let alpha_before = s.active().unwrap().doc.layers[0].surface().unwrap().pixel(0, 0).to_vec();
        match s.start(CMD, json!({"variant": "hr"})) {
            Ok(Started::Job(job)) => {
                let e = s.execute("layer.new.layer", json!({})).unwrap_err();
                assert!(e.to_string().contains("still running"), "edits are locked while the job runs: {e}");
                assert!(s.cancel_job(job), "the job is cancellable");
                s.join_cancelled_jobs();
                let events = s.poll_jobs();
                assert!(events.iter().any(|ev| ev.id == job && matches!(ev.outcome, crate::jobs::JobOutcome::Cancelled)), "{events:?}");
            }
            Ok(Started::Done(v)) => {
                // Inline fallback (wasm): the command still answers, and the
                // "hr" weights are absent here, so nothing was applied.
                assert!(v.is_null() || v.is_object(), "{v}");
            }
            #[cfg(target_arch = "wasm32")]
            Err(_) => {
                // On wasm the job runs inline and matting is compiled out, so
                // the command answers UnsupportedOnWasm; that is the expected
                // outcome rather than a panic.
            }
            #[cfg(not(target_arch = "wasm32"))]
            Err(e) => panic!("start failed: {e}"),
        }
        let alpha_after = s.active().unwrap().doc.layers[0].surface().unwrap().pixel(0, 0).to_vec();
        assert_eq!(alpha_after, alpha_before, "a cancelled run leaves the document untouched");
    }
}
