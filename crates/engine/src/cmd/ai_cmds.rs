//! Current visible canvas + instruction -> provider -> one undoable top-level layer.
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};
#[cfg(not(target_arch = "wasm32"))]
use photocraft_doc::{Document, Layer, LayerContent};
#[cfg(not(target_arch = "wasm32"))]
use photocraft_gen::{Error as ProviderError, ImageProvider};
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

pub const COMMAND: &str = "edit.ai.codex";
mod png;

fn enabled(s: &Session) -> std::result::Result<(), String> {
    if cfg!(target_arch = "wasm32") {
        return Err("AI editing requires the desktop app".into());
    }
    let d = s.active().ok_or("no document open")?;
    if d.doc.size.width == 0 || d.doc.size.height == 0 {
        return Err("AI editing requires a non-empty canvas".into());
    }
    if !crate::layer_copy_cmds::layered(d.doc.mode) {
        return Err("AI editing currently supports RGB, Gray, CMYK and Lab documents".into());
    }
    if u64::from(d.doc.size.width) * u64::from(d.doc.size.height) > png::MAX_PIXELS {
        return Err("AI prototype supports canvases up to 16 megapixels".into());
    }
    Ok(())
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let prompt = p
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty() && p.len() <= 16_000)
        .ok_or_else(|| EngineError::BadParams { cmd: COMMAND.into(), msg: "prompt must contain 1–16000 bytes of text".into() })?
        .to_string();
    #[cfg(not(target_arch = "wasm32"))]
    {
        edit_with_provider(s, prompt, Arc::new(photocraft_gen::codex::CodexProvider::default()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, prompt);
        Err(EngineError::Other("AI editing requires the desktop Codex runtime".into()))
    }
}

/// Injection seam for alternative services and deterministic offline tests.
#[cfg(not(target_arch = "wasm32"))]
pub fn edit_with_provider(s: &mut Session, prompt: String, provider: Arc<dyn ImageProvider>) -> Result<Value> {
    if prompt.trim().is_empty() || prompt.len() > 16_000 {
        return Err(EngineError::BadParams { cmd: COMMAND.into(), msg: "prompt must contain 1–16000 bytes of text".into() });
    }
    enabled(s).map_err(EngineError::Other)?;
    let snapshot = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let intent = s.color.settings.intent();
    let bpc = s.color.settings.bpc;
    let label = "Codex";
    crate::jobs::run(
        s,
        "AI Edit",
        true,
        move |ctx| {
            if ctx.cancelled() {
                return Err(EngineError::Cancelled);
            }
            ctx.progress(0.05, "Preparing visible canvas");
            let work = tempfile::Builder::new().prefix("photocraft-ai-").tempdir().map_err(|e| EngineError::Other(e.to_string()))?;
            // A fresh document deliberately omits EXIF, XMP, hidden layers and document names.
            let mut input = Document::new("Canvas", snapshot.size, snapshot.mode, snapshot.depth);
            input.icc_profile = snapshot.icc_profile.clone();
            let surface = photocraft_compose::flatten_to_surface(&snapshot, snapshot.pixel_format(), None);
            input.layers.push(Layer::new("Canvas", LayerContent::Raster(surface)));
            crate::color_cmds::convert_document(&mut input, photocraft_cms::builtin::srgb(), intent, bpc)?;
            crate::image_cmds::for_each_surface(&mut input.layers, true, &mut |surface, _| {
                *surface = surface.convert(surface.format().with_sample(photocraft_color::SampleType::U8));
            });
            input.depth = photocraft_color::SampleType::U8;
            let opts = photocraft_io::ExportOptions { xmp: photocraft_io::XmpEmbed::None, ..Default::default() };
            let png = photocraft_io::export(&input, "png", &opts).map_err(|e| EngineError::Other(e.to_string()))?;
            let image = work.path().join("canvas.png");
            std::fs::write(&image, png.bytes).map_err(|e| EngineError::Other(e.to_string()))?;
            ctx.progress(0.15, &format!("Editing through {label}"));
            let result = provider.edit(&photocraft_gen::EditRequest { image: &image, prompt: &prompt, workspace: work.path() }, &|| ctx.cancelled()).map_err(
                |e| match e {
                    ProviderError::Cancelled => EngineError::Cancelled,
                    e => EngineError::Other(e.to_string()),
                },
            )?;
            if ctx.cancelled() {
                return Err(EngineError::Cancelled);
            }
            ctx.progress(0.9, "Importing edited image");
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(result.saved_path)
                .and_then(|f| f.take(png::MAX_FILE_BYTES as u64 + 1).read_to_end(&mut bytes))
                .map_err(|e| EngineError::Other(e.to_string()))?;
            let (source_width, source_height) =
                png::result_dimensions(&bytes, snapshot.size.width, snapshot.size.height).map_err(|msg| EngineError::Other(msg.into()))?;
            let filename = if bytes.starts_with(b"\xff\xd8\xff") { "ai-result.jpg" } else { "ai-result.png" };
            let mut output = photocraft_io::import(filename, &bytes).map_err(|e| EngineError::Other(e.to_string()))?.document;
            if output.size.width != source_width || output.size.height != source_height {
                return Err(EngineError::Other("AI result decoded dimensions differ from its image header; no layer was added".into()));
            }
            let resized = output.size != snapshot.size;
            if resized {
                ctx.progress(0.95, "Fitting result to canvas");
                let canvas = output.bounds();
                let sx = f64::from(snapshot.size.width) / f64::from(source_width);
                let sy = f64::from(snapshot.size.height) / f64::from(source_height);
                crate::image_cmds::for_each_surface(&mut output.layers, true, &mut |surface, _| {
                    *surface = photocraft_algo::resample::resize_surface_in_canvas(surface, sx, sy, photocraft_algo::resample::Resample::Bicubic, canvas);
                });
                output.size = snapshot.size;
            }
            if ctx.cancelled() {
                return Err(EngineError::Cancelled);
            }
            Ok((output, resized, source_width, source_height))
        },
        move |s, (mut output, resized, source_width, source_height)| {
            let dest = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
            crate::layer_copy_cmds::adapt_copies(s, &mut output, &dest)?;
            let surface = photocraft_compose::flatten_to_surface(&output, dest.pixel_format(), None);
            let layer = Layer::new(format!("AI Edit ({label})"), LayerContent::Raster(surface));
            let id = layer.id;
            s.edit("AI Edit", move |doc, active| {
                // Full-canvas output goes on top, regardless of the selected group or layer lock.
                doc.layers.push(layer);
                *active = Some(id);
                Ok(())
            })?;
            s.select_layer(id)?;
            Ok(json!({"layer":id.0,"provider":"codex","resizedToCanvas":resized,"sourceWidth":source_width,"sourceHeight":source_height}))
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "AI Edit…",
        menu: &["Edit"],
        shortcut: None,
        params: r#"{"prompt":"instruction"} -> {layer,provider}; sends visible canvas to installed Codex using ChatGPT sign-in; desktop only, no API fallback"#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    struct Echo;

    impl ImageProvider for Echo {
        fn edit(&self, r: &photocraft_gen::EditRequest<'_>, _: &dyn Fn() -> bool) -> std::result::Result<photocraft_gen::EditResult, ProviderError> {
            assert_eq!(r.prompt, "test edit");
            Ok(photocraft_gen::EditResult { saved_path: r.image.to_path_buf() })
        }
    }
    struct Fail;
    impl ImageProvider for Fail {
        fn edit(&self, _: &photocraft_gen::EditRequest<'_>, _: &dyn Fn() -> bool) -> std::result::Result<photocraft_gen::EditResult, ProviderError> {
            Err(ProviderError::Provider("test unavailable".into()))
        }
    }
    struct ScaledResult;
    impl ImageProvider for ScaledResult {
        fn edit(&self, r: &photocraft_gen::EditRequest<'_>, _: &dyn Fn() -> bool) -> std::result::Result<photocraft_gen::EditResult, ProviderError> {
            let mut source = Session::new();
            source.execute("file.new", json!({"width":16,"height":12,"background":"#ff0000"})).map_err(|e| ProviderError::Provider(e.to_string()))?;
            let doc = &source.active().ok_or_else(|| ProviderError::Provider("test document unavailable".into()))?.doc;
            let png = photocraft_io::export(doc, "png", &Default::default()).map_err(|e| ProviderError::Provider(e.to_string()))?;
            let saved_path = r.workspace.join("scaled-output.png");
            std::fs::write(&saved_path, png.bytes)?;
            Ok(photocraft_gen::EditResult { saved_path })
        }
    }
    #[test]
    fn same_aspect_output_is_fitted_and_undoable() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":6})).unwrap();
        let result = edit_with_provider(&mut s, "test edit".into(), Arc::new(ScaledResult)).unwrap();
        assert_eq!(result["resizedToCanvas"], true);
        assert_eq!(result["sourceWidth"], 16);
        assert_eq!(result["sourceHeight"], 12);
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.size, photocraft_doc::Size::new(8, 6));
        let pixel = doc.layers[1].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 1, 1));
        assert!(pixel[0] > 0.99 && pixel[1] < 0.01 && pixel[2] < 0.01);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    }
    #[test]
    fn new_layer_preserves_depth_and_undo_redo() {
        for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("gray", 16), ("gray", 32), ("cmyk", 8), ("cmyk", 16), ("lab", 8), ("lab", 16)]
        {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":8,"height":6,"depth":depth,"mode":mode})).unwrap();
            let original = s.active().unwrap().doc.layers[0].id;
            let before = s.active().unwrap().doc.layers[0].surface().unwrap().read_region(s.active().unwrap().doc.bounds());
            let result = edit_with_provider(&mut s, "test edit".into(), Arc::new(Echo)).unwrap();
            let d = &s.active().unwrap().doc;
            assert_eq!(d.layers.len(), 2);
            assert_eq!(d.layers[0].id, original);
            assert_eq!(d.layers[0].surface().unwrap().read_region(d.bounds()), before);
            assert_eq!(d.layers[1].surface().unwrap().format().sample, d.depth);
            assert_eq!(d.layers[1].surface().unwrap().format().mode, d.mode);
            assert_eq!(s.active().unwrap().active_layer.unwrap().0, result["layer"]);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers.len(), 1);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers.len(), 2);
        }
    }
    #[test]
    fn failures_and_invalid_params_leave_document_intact() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":6})).unwrap();
        assert!(edit_with_provider(&mut s, "test edit".into(), Arc::new(Fail)).is_err());
        for prompt in [json!(null), json!(""), json!(" "), json!(42)] {
            assert!(s.execute(COMMAND, json!({"prompt":prompt})).is_err());
        }
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert!(png::dimensions(b"bad", 8, 6).is_err());
    }
    #[test]
    fn cancellation_before_provider_call_adds_no_layer() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":6})).unwrap();
        let ctx = crate::jobs::JobCtx::new();
        ctx.cancel();
        s.set_inline_job_ctx(Some(ctx));
        assert!(matches!(edit_with_provider(&mut s, "test edit".into(), Arc::new(Echo)), Err(EngineError::Cancelled)));
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    }
    #[test]
    fn rejects_wrong_size_png_before_decode() {
        let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        header.extend_from_slice(&8u32.to_be_bytes());
        header.extend_from_slice(&6u32.to_be_bytes());
        assert!(png::dimensions(&header, 8, 6).is_ok());
        assert!(png::dimensions(&header, 9, 6).is_err());
        assert!(png::dimensions(&header, 8, 9).is_err());
    }
}
