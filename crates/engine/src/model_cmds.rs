//! Optional model lifecycle commands and the colour/depth-agnostic selection adapter.
use std::sync::Arc;

use photocraft_algo::segment::{Sampler, antialias_u8, trim_region};
use photocraft_algo::selection::Region;
use photocraft_cms::{Builtin, Transform, transform::TransformOptions};
use photocraft_doc::Document;
use photocraft_geom::Rect;
use photocraft_ml::{InferenceBackend, MaskKind, ModelId, Prompt};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::jobs::JobCtx;
use crate::{EngineError, Result, Session};

fn error(e: photocraft_ml::Error) -> EngineError {
    match e {
        photocraft_ml::Error::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(e.to_string()),
    }
}

pub(crate) fn backend(s: &Session) -> Result<Arc<dyn InferenceBackend>> {
    s.model_backend
        .clone()
        .ok_or_else(|| EngineError::Other("local models are not available in this build or session; choose Classical in Preferences › Integrations".into()))
}

/// An explicit command choice overrides the saved preference, allowing an action or script to
/// request the same method every time. Inference never silently falls back or starts a download.
pub(crate) fn subject_model(s: &Session, p: &Value) -> Result<Option<ModelId>> {
    model_choice(p, s.prefs().integrations.subject_model.name(), ModelId::BiRefNet)
}

pub(crate) fn object_model(s: &Session, p: &Value) -> Result<Option<ModelId>> {
    model_choice(p, s.prefs().integrations.object_model.name(), ModelId::Sam2)
}

/// Resolve the preference before dispatch so recorded actions keep the method used, even if
/// the preference changes before playback. Explicit choices (including invalid ones) stay intact.
pub(crate) fn inject_model(s: &Session, command: &str, mut params: Value) -> Value {
    let model = match command {
        "select.subject" | "layer.removeBackground" => s.prefs().integrations.subject_model.name(),
        "select.object" => s.prefs().integrations.object_model.name(),
        _ => return params,
    };
    if params.is_null() {
        params = json!({});
    }
    if let Some(fields) = params.as_object_mut() {
        fields.entry("model").or_insert_with(|| json!(model));
    }
    params
}

fn model_choice(p: &Value, default: &str, allowed: ModelId) -> Result<Option<ModelId>> {
    let name = match p.get("model") {
        Some(v) => v.as_str().ok_or_else(|| EngineError::Other("model must be a string".into()))?,
        None => default,
    };
    if name == "classical" {
        return Ok(None);
    }
    match ModelId::parse(name) {
        Some(id) if id == allowed => Ok(Some(id)),
        _ => Err(EngineError::Other(format!("model must be classical or {} for this command", allowed.name()))),
    }
}

struct ModelSampler<'a> {
    source: &'a dyn Sampler,
    cmyk: Option<Arc<photocraft_color::convert::CmykSpace>>,
    transform: Arc<Transform>,
}

impl Sampler for ModelSampler<'_> {
    fn rgba(&self, rect: Rect) -> Vec<[f32; 4]> {
        // rgb_scaled reads strips on rayon workers. Enter the document's CMYK context on each
        // worker, and convert its RGB/gray profile before downsampling or compositing transparency.
        let mut px = photocraft_color::convert::with_cmyk_space(self.cmyk.as_ref(), || self.source.rgba(rect));
        for p in &mut px {
            let mut rgb = [0.0; 3];
            self.transform.eval(&p[..3], &mut rgb);
            for (dst, v) in p.iter_mut().take(3).zip(rgb) {
                *dst = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            }
            if !p[3].is_finite() {
                p[3] = 0.0;
            }
        }
        px
    }
}

pub(crate) fn infer_region(
    backend: &dyn InferenceBackend,
    id: ModelId,
    source: &dyn Sampler,
    doc: &Document,
    prompt: Prompt,
    ctx: &JobCtx,
) -> Result<Option<Region>> {
    let bounds = doc.bounds();
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);
    let n = w
        .checked_mul(h)
        .filter(|n| *n > 0 && *n <= 64_000_000)
        .ok_or_else(|| EngineError::Other("local models currently support canvases up to 64 megapixels".into()))?;
    ctx.check()?;
    ctx.progress(0.02, "Preparing image");
    let transform = photocraft_cms::transform::cached(&crate::color_cmds::composite_profile(doc), Builtin::Srgb.profile(), TransformOptions::default())
        .map_err(|e| EngineError::Other(format!("model colour conversion: {e}")))?;
    let sampler = ModelSampler { source, cmyk: photocraft_compose::cmyk_space(doc), transform };
    let step = w.max(h).div_ceil(id.info().input_side).max(1);
    let image = sampler.rgb_scaled(bounds, step);
    ctx.check()?;
    ctx.progress(0.1, "Running local model (CPU)");
    let mask = ctx.stage(0.1, 0.85, "Running local model (CPU)", |ctl| backend.infer(id, &image, prompt, ctl)).map_err(error)?;
    mask.validate().map_err(error)?;
    let mut values = vec![0u8; n];
    for (y, row) in values.chunks_exact_mut(w).enumerate() {
        ctx.check()?;
        for (x, dst) in row.iter_mut().enumerate() {
            *dst = mask.coverage((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
        }
        if y % 64 == 0 {
            ctx.progress(0.85 + 0.14 * y as f32 / h as f32, "Building mask");
        }
    }
    if mask.kind == MaskKind::Logits {
        antialias_u8(&mut values, w, h);
    }
    ctx.check()?;
    Ok(trim_region(Region { bbox: bounds, mask: values }))
}

impl Session {
    /// Inject a native model cache. Host code supplies the location; no arbitrary model path or
    /// URL is accepted by an engine command. Default builds and wasm have no native backend.
    #[cfg(all(feature = "local-ml", not(target_arch = "wasm32")))]
    pub fn configure_local_models(&mut self, directory: std::path::PathBuf) {
        self.model_backend = Some(Arc::new(photocraft_ml::NativeBackend::new(directory)));
    }

    pub fn local_model_status(&self) -> Value {
        let states = self.model_backend.as_ref().map(|b| b.status()).unwrap_or_default();
        let models: Vec<Value> = photocraft_ml::MODELS
            .iter()
            .map(|m| {
                let status = states.iter().find(|s| s.id == m.id);
                json!({"id": m.id, "label": m.label, "purpose": m.purpose, "license": m.license,
                "downloadBytes": m.download_bytes(), "upstream": m.upstream, "exportSource": m.export_source,
                "revision": m.revision, "installed": status.is_some_and(|s| s.installed), "busy": status.is_some_and(|s| s.busy)})
            })
            .collect();
        json!({"available": self.model_backend.is_some(), "models": models})
    }
}

fn model_id(p: &Value, cmd: &str) -> Result<ModelId> {
    p.get("id")
        .and_then(Value::as_str)
        .and_then(ModelId::parse)
        .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "id must be birefnet-hr-matting or sam2.1-large".into() })
}

fn available(s: &Session) -> std::result::Result<(), String> {
    if s.model_backend.is_some() { Ok(()) } else { Err("local models are not available in this build or session".into()) }
}

fn download(s: &mut Session, p: &Value) -> Result<Value> {
    let id = model_id(p, "models.download")?;
    let backend = backend(s)?;
    crate::jobs::run(
        s,
        &format!("Downloading {}", id.info().label),
        false,
        move |ctx| ctx.stage(0.0, 1.0, "Downloading model", |ctl| backend.download(id, ctl)).map_err(error),
        move |s, ()| Ok(s.local_model_status()),
    )
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = model_id(p, "models.remove")?;
    let backend = backend(s)?;
    crate::jobs::run(
        s,
        &format!("Removing {}", id.info().label),
        false,
        move |ctx| ctx.stage(0.0, 1.0, "Removing model", |ctl| backend.remove(id, ctl)).map_err(error),
        move |s, ()| {
            s.prefs.edit(|p| match id {
                ModelId::BiRefNet => p.integrations.subject_model = crate::prefs::SubjectModel::Classical,
                ModelId::Sam2 => p.integrations.object_model = crate::prefs::ObjectModel::Classical,
            });
            Ok(s.local_model_status())
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "models.list",
            label: "Local Selection Models",
            menu: &[],
            shortcut: None,
            params: "{} → {available,models:[{id,label,installed,busy,downloadBytes,license,revision,upstream,exportSource}]}",
            enabled: |_| Ok(()),
            run: |s, _| Ok(s.local_model_status()),
            journal: false,
        },
        CommandSpec {
            id: "models.download",
            label: "Download Local Selection Model",
            menu: &[],
            shortcut: None,
            params: r#"{"id":"birefnet-hr-matting|sam2.1-large"} (explicit download; size/SHA-256 verified; no document edits)"#,
            enabled: available,
            run: download,
            journal: false,
        },
        CommandSpec {
            id: "models.remove",
            label: "Remove Local Selection Model",
            menu: &[],
            shortcut: None,
            params: r#"{"id":"birefnet-hr-matting|sam2.1-large"} (remove only this model; restore its selection preference to classical)"#,
            enabled: available,
            run: remove,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::segment::RgbImage;
    use photocraft_ml::{AlphaMask, ModelStatus};
    use photocraft_raster::Interrupt;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Fake {
        installed: AtomicBool,
        downloads: AtomicUsize,
        block: AtomicBool,
        entered: AtomicBool,
        calls: Mutex<Vec<(ModelId, RgbImage, Prompt)>>,
    }
    impl InferenceBackend for Fake {
        fn status(&self) -> Vec<ModelStatus> {
            photocraft_ml::MODELS.iter().map(|m| ModelStatus { id: m.id, installed: self.installed.load(Ordering::Relaxed), busy: false }).collect()
        }
        fn download(&self, _: ModelId, _: &Interrupt<'_>) -> photocraft_ml::Result<()> {
            self.downloads.fetch_add(1, Ordering::Relaxed);
            self.installed.store(true, Ordering::Relaxed);
            Ok(())
        }
        fn remove(&self, _: ModelId, _: &Interrupt<'_>) -> photocraft_ml::Result<()> {
            self.installed.store(false, Ordering::Relaxed);
            Ok(())
        }
        fn infer(&self, id: ModelId, image: &RgbImage, prompt: Prompt, ctl: &Interrupt<'_>) -> photocraft_ml::Result<AlphaMask> {
            self.entered.store(true, Ordering::Relaxed);
            if !self.installed.load(Ordering::Relaxed) {
                return Err(photocraft_ml::Error::Unavailable("download first".into()));
            }
            self.calls.lock().unwrap().push((id, RgbImage { w: image.w, h: image.h, px: image.px.clone() }, prompt));
            if self.block.load(Ordering::Relaxed) {
                for _ in 0..400 {
                    if ctl.cancelled() {
                        return Err(photocraft_ml::Error::Cancelled);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                return Err(photocraft_ml::Error::Inference("test backend timed out".into()));
            }
            Ok(AlphaMask { width: 2, height: 1, values: vec![0.0, 1.0], kind: MaskKind::Alpha })
        }
    }
    fn session(mode: &str, depth: u32) -> (Session, Arc<Fake>) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":4,"depth":depth,"mode":mode})).unwrap();
        let fake = Arc::new(Fake::default());
        fake.installed.store(true, Ordering::Relaxed);
        s.model_backend = Some(fake.clone());
        (s, fake)
    }
    fn cov(s: &Session, x: i32) -> f32 {
        s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, 2, 0))
    }
    fn pixels(s: &Session) -> Vec<f32> {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(d.doc.bounds())
    }

    #[test]
    fn learned_background_mask_preserves_pixels_soft_alpha_and_one_undo_at_all_depths_and_modes() {
        for mode in ["rgb", "gray", "cmyk", "lab"] {
            for depth in [8, 16, 32] {
                let (mut s, fake) = session(mode, depth);
                s.execute("select.rect", json!({"x":0,"y":0,"width":2,"height":4})).unwrap();
                let old_selection = s.active().unwrap().doc.selection.clone();
                let before = pixels(&s);
                let steps = s.active().unwrap().history.past_len();
                s.execute("layer.removeBackground", json!({"model":"birefnet-hr-matting"})).unwrap();
                let d = s.active().unwrap();
                let mask = d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap();
                assert_eq!(mask.value(0, 2), 0.0);
                assert_eq!(mask.value(7, 2), 1.0);
                assert!((mask.value(3, 2) - 0.376).abs() < 0.01);
                assert_eq!(pixels(&s), before, "{mode} {depth}");
                assert!(d.doc.selection.is_none());
                assert_eq!(d.history.past_len(), steps + 1);
                let calls = fake.calls.lock().unwrap();
                assert_eq!((calls[0].1.w, calls[0].1.h), (8, 4));
                assert!(calls[0].1.px.iter().flatten().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
                drop(calls);
                s.execute("edit.undo", json!({})).unwrap();
                let d = s.active().unwrap();
                assert!(d.doc.layer(d.active_layer.unwrap()).unwrap().mask.is_none());
                assert_eq!(d.doc.selection.as_ref().unwrap().read_region(d.doc.bounds()), old_selection.as_ref().unwrap().read_region(d.doc.bounds()));
                s.execute("edit.redo", json!({})).unwrap();
                assert!(s.active().unwrap().doc.layers[0].mask.is_some());
            }
        }
    }

    #[test]
    fn learned_selection_combines_modes_and_maps_boxes_on_non_square_images() {
        for (mode, left, right) in [("replace", false, true), ("add", true, true), ("subtract", true, false), ("intersect", false, false)] {
            let (mut s, fake) = session("rgb", 16);
            s.execute("select.rect", json!({"x":0,"y":0,"width":2,"height":4})).unwrap();
            let steps = s.active().unwrap().history.past_len();
            s.execute("select.object", json!({"rect":[6,3,-4,-2],"model":"sam2.1-large","mode":mode})).unwrap();
            assert_eq!(cov(&s, 0) > 0.5, left, "{mode}");
            assert_eq!(cov(&s, 7) > 0.5, right, "{mode}");
            let calls = fake.calls.lock().unwrap();
            assert_eq!(calls[0].2, Prompt::Box([0.25, 0.25, 0.75, 0.75]));
            drop(calls);
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(cov(&s, 0), 1.0);
            assert_eq!(cov(&s, 7), 0.0);
        }
    }

    #[test]
    fn missing_models_invalid_choices_and_bad_boxes_never_edit_or_download() {
        let (mut s, fake) = session("rgb", 8);
        fake.installed.store(false, Ordering::Relaxed);
        let revision = s.active().unwrap().revision;
        for (cmd, p) in [
            ("select.subject", json!({"model":"birefnet-hr-matting"})),
            ("layer.removeBackground", json!({"model":"birefnet-hr-matting"})),
            ("select.object", json!({"model":"sam2.1-large","rect":[0,0,8,4]})),
            ("select.subject", json!({"model":"sam2.1-large"})),
            ("select.subject", json!({"model":42})),
            ("models.download", json!({"id":"../unsafe"})),
            ("models.remove", json!({"id":42})),
        ] {
            assert!(s.execute(cmd, p).is_err(), "{cmd}");
        }
        for rect in [json!([0, 0, "bad", 8, 4]), json!([0, 0, 8]), json!([1e100, 0, 8, 4]), json!([0, 0, 0, 0]), json!([100, 100, 8, 4])] {
            assert!(s.execute("select.object", json!({"model":"sam2.1-large","rect":rect})).is_err());
        }
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(fake.downloads.load(Ordering::Relaxed), 0);
        s.model_backend = None;
        assert_eq!(s.execute("models.list", json!({})).unwrap()["available"], false);
        assert!(s.execute("select.subject", json!({"model":"birefnet-hr-matting"})).is_err());
    }

    #[test]
    fn downloads_need_no_document_never_replay_and_removal_resets_only_its_preference() {
        let mut s = Session::new();
        let fake = Arc::new(Fake::default());
        s.model_backend = Some(fake.clone());
        s.execute("models.download", json!({"id":"birefnet-hr-matting"})).unwrap();
        assert!(s.journal.is_empty());
        assert_eq!(fake.downloads.load(Ordering::Relaxed), 1);
        s.execute("prefs.set", json!({"values":{"integrations.subjectModel":"birefnet-hr-matting","integrations.objectModel":"sam2.1-large"}})).unwrap();
        s.execute("models.remove", json!({"id":"birefnet-hr-matting"})).unwrap();
        assert_eq!(s.prefs().integrations.subject_model, crate::prefs::SubjectModel::Classical);
        assert_eq!(s.prefs().integrations.object_model, crate::prefs::ObjectModel::Sam2);
        assert!(s.execute("prefs.set", json!({"path":"integrations.objectModel","value":"birefnet-hr-matting"})).is_err());
        let mut old = s.prefs_value();
        old["integrations"].as_object_mut().unwrap().remove("objectModel");
        old["integrations"].as_object_mut().unwrap().remove("subjectModel");
        let loaded: crate::prefs::Preferences = serde_json::from_value(old).unwrap();
        assert_eq!(loaded.integrations.subject_model, crate::prefs::SubjectModel::Classical);
        assert_eq!(loaded.integrations.object_model, crate::prefs::ObjectModel::Classical);
    }

    #[test]
    fn preferences_select_a_model_and_explicit_classical_overrides_it() {
        let (mut s, fake) = session("rgb", 32);
        s.execute("prefs.set", json!({"path":"integrations.subjectModel","value":"birefnet-hr-matting"})).unwrap();
        s.execute("select.subject", json!({})).unwrap();
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
        s.execute("select.subject", json!({"model":"classical"})).unwrap();
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn recorded_actions_keep_the_selected_method_when_preferences_change() {
        let (mut s, fake) = session("rgb", 8);
        s.edit_prefs(|p| {
            p.integrations.subject_model = crate::prefs::SubjectModel::BiRefNet;
            p.integrations.object_model = crate::prefs::ObjectModel::Sam2;
        });
        s.execute("actions.record", json!({"name":"Models"})).unwrap();
        s.execute("select.subject", Value::Null).unwrap();
        s.execute("select.object", json!({"rect":[0,0,8,4]})).unwrap();
        s.execute("layer.removeBackground", json!({})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        let action = &s.actions.list[0];
        assert_eq!(action.steps.len(), 3);
        assert_eq!(action.steps[0].1["model"], "birefnet-hr-matting");
        assert_eq!(action.steps[1].1["model"], "sam2.1-large");
        assert_eq!(action.steps[2].1["model"], "birefnet-hr-matting");
        s.edit_prefs(|p| p.integrations = Default::default());
        assert!(s.execute("actions.play", json!({"action":"Models"})).unwrap().get("failed").is_none());
        assert_eq!(fake.calls.lock().unwrap().len(), 6);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn batch_and_droplet_share_only_the_explicitly_configured_model_capability() {
        let dir = std::env::temp_dir().join(format!("photocraft-model-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let input = dir.join("input.png");
        let output = dir.join("output");
        std::fs::create_dir(&output).unwrap();
        let (mut s, fake) = session("rgb", 8);
        let exported = photocraft_io::export(&s.active().unwrap().doc, "input.png", &Default::default()).unwrap();
        std::fs::write(&input, exported.bytes).unwrap();
        s.edit_prefs(|p| p.integrations.subject_model = crate::prefs::SubjectModel::BiRefNet);
        let droplet = dir.join("models.pcdroplet");
        std::fs::write(
            &droplet,
            serde_json::to_vec(&json!({"photocraftDroplet":1,"action":{"steps":[["layer.removeBackground",{}]]},"options":{"format":"png"}})).unwrap(),
        )
        .unwrap();
        let result = s.execute("file.automate.runDroplet", json!({"droplet":droplet,"input":[input],"output":output})).unwrap();
        assert_eq!(result["errors"], json!([]));
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
        s.model_backend = None;
        let result =
            s.execute("file.automate.batch", json!({"steps":[["select.subject",{"model":"birefnet-hr-matting"}]],"input":[input],"output":output})).unwrap();
        assert_eq!(result["errors"].as_array().unwrap().len(), 1);
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn model_input_uses_document_profile_and_existing_mask_replacement_undo_is_preserved() {
        let (mut s, fake) = session("rgb", 32);
        s.edit("linear pixels and old mask", |doc, _| {
            doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
            let bounds = doc.bounds();
            doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &[0.25, 0.25, 0.25, 1.0]);
            doc.layers[0].mask = Some(photocraft_doc::LayerMask::hide_all());
            Ok(())
        })
        .unwrap();
        s.execute("select.subject", json!({"model":"birefnet-hr-matting","sampleAllLayers":false})).unwrap();
        let calls = fake.calls.lock().unwrap();
        assert!((calls[0].1.px[0][0] - 0.5371).abs() < 0.005, "linear RGB must be transformed to sRGB");
        drop(calls);
        s.execute("layer.removeBackground", json!({"model":"birefnet-hr-matting"})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers[0].mask.as_ref().unwrap().value(7, 2), 1.0);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers[0].mask.as_ref().unwrap().value(7, 2), 0.0);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn cancelling_model_inference_discards_result_and_unlocks_document() {
        let (mut s, fake) = session("rgb", 8);
        fake.block.store(true, Ordering::Relaxed);
        let before = s.active().unwrap().revision;
        let crate::jobs::Started::Job(job) = s.start("layer.removeBackground", json!({"model":"birefnet-hr-matting"})).unwrap() else { panic!("expected job") };
        for _ in 0..200 {
            if fake.entered.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(fake.entered.load(Ordering::Relaxed));
        assert!(s.cancel_job(job));
        assert!(matches!(s.wait_job(job), Err(EngineError::Cancelled)));
        assert_eq!(s.active().unwrap().revision, before);
        assert!(s.active().unwrap().doc.layers[0].mask.is_none());
        s.execute("select.all", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_some());
    }
}
