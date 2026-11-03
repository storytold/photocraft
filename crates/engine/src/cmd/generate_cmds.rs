//! Native image generation through the installed per-user Enki runtime.

use crate::{EngineError, Result, Session, commands::CommandSpec};
#[cfg(target_os = "linux")]
use photocraft_doc::{Layer, LayerContent};
use serde_json::Value;
#[cfg(target_os = "linux")]
use serde_json::json;

#[cfg(target_os = "linux")]
mod edits;
#[cfg(target_os = "linux")]
mod subject;
#[cfg(all(test, target_os = "linux"))]
mod tests;
#[cfg(target_os = "linux")]
mod transport;

pub const COMMAND: &str = "image.generate";
#[cfg(target_os = "linux")]
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct Request {
    prompt: String,
    negative: String,
    model: String,
    width: u32,
    height: u32,
    steps: Option<u32>,
    cfg: Option<f64>,
    seed: i64,
    strength: Option<f64>,
    edit: Option<EditInput>,
    dry_run: bool,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct EditInput {
    source_png: String,
    mask_png: String,
}

#[cfg(target_os = "linux")]
impl Request {
    fn parse(p: &Value) -> Result<Self> {
        if !p.is_object() {
            return Err(bad("params must be an object"));
        }
        let text = |key: &str, default: &str, max: usize| -> Result<String> {
            let value = match p.get(key) {
                None => default,
                Some(v) => v.as_str().ok_or_else(|| bad(format!("{key} must be text")))?,
            };
            if value.chars().count() > max {
                return Err(bad(format!("{key} exceeds {max} characters")));
            }
            Ok(value.into())
        };
        let prompt = text("prompt", "", 2000)?;
        if prompt.trim().is_empty() {
            return Err(bad("enter a prompt"));
        }
        let dimension = |key: &str| -> Result<u32> {
            let n = p.get(key).map_or(Some(512), Value::as_u64).ok_or_else(|| bad(format!("{key} must be an integer")))?;
            if !(64..=1024).contains(&n) || n % 8 != 0 {
                return Err(bad(format!("{key} must be 64–1024 and a multiple of 8")));
            }
            Ok(n as u32)
        };
        let steps =
            p.get("steps").map(|v| v.as_u64().filter(|n| (1..=50).contains(n)).map(|n| n as u32).ok_or_else(|| bad("steps must be 1–50"))).transpose()?;
        let cfg = p.get("cfg").map(|v| v.as_f64().filter(|n| n.is_finite() && (1.0..=20.0).contains(n)).ok_or_else(|| bad("cfg must be 1–20"))).transpose()?;
        let seed = p.get("seed").map_or(Ok(-1), |v| v.as_i64().ok_or_else(|| bad("seed must be a signed integer")))?;
        Ok(Self {
            prompt,
            negative: text("negative", "", 2000)?,
            model: text("model", "", 256)?,
            width: dimension("width")?,
            height: dimension("height")?,
            steps,
            cfg,
            seed,
            strength: p
                .get("strength")
                .map(|v| v.as_f64().filter(|n| n.is_finite() && (0.05..=1.0).contains(n)).ok_or_else(|| bad("strength must be 0.05–1")))
                .transpose()?,
            edit: None,
            dry_run: p.get("dryRun").map_or(Ok(false), |v| v.as_bool().ok_or_else(|| bad("dryRun must be boolean")))?,
        })
    }
}

// shortcut: Linux peer verification only; add native transports before enabling other platforms.
fn enabled(s: &Session) -> std::result::Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("Enki connection is currently available on Linux only".into());
    }
    let st = s.active().ok_or("open a document first")?;
    if !matches!(
        st.doc.mode,
        photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale | photocraft_color::ColorMode::Cmyk | photocraft_color::ColorMode::Lab
    ) {
        return Err("Generate Image requires an RGB, grayscale, CMYK or Lab document".into());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn run(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("Enki connection is currently available on Linux only".into()))
}

#[cfg(target_os = "linux")]
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let request = Request::parse(p)?;
    enabled(s).map_err(EngineError::Other)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if p.get("document").is_some_and(|v| v.as_u64() != Some(st.doc.id.0)) {
        return Err(bad("the target document is no longer active"));
    }
    let background = match p.get("placement").and_then(Value::as_str) {
        None if p.get("placement").is_none() => None,
        Some("layer") => None,
        Some("background") => {
            let id = st.active_layer.ok_or_else(|| bad("select a masked subject layer"))?;
            super::background_fill_cmds::subject(s, id).map_err(EngineError::Other)?;
            if st.doc.size.width > 1024 || st.doc.size.height > 1024 {
                return Err(bad("this runtime supports backgrounds up to 1024 pixels per side; larger canvases require tiled generation"));
            }
            if request.width < st.doc.size.width || request.height < st.doc.size.height {
                return Err(bad("a generated background must cover the canvas; increase the requested dimensions"));
            }
            Some(id)
        }
        _ => return Err(bad("placement must be layer or background")),
    };
    if request.dry_run {
        return Ok(json!({"dryRun":true,"width":request.width,"height":request.height,"placement":if background.is_some(){"background"}else{"layer"}}));
    }
    let profile = crate::color_cmds::document_profile(&st.doc);
    let format = st.doc.pixel_format();
    let intent = s.color.settings.intent();
    let bpc = s.color.settings.bpc;
    crate::jobs::run(
        s,
        "Generate Image",
        true,
        move |ctx| {
            ctx.check()?;
            let output = transport::generate(&request, ctx)?;
            let surface = decode(&output, &profile, format, intent, bpc)?;
            ctx.check()?;
            Ok((surface, output.seed, output.model))
        },
        move |s, (surface, seed, model)| apply(s, surface, seed, &model, background),
    )
}

#[cfg(target_os = "linux")]
fn decode(
    output: &Output,
    profile: &photocraft_cms::Profile,
    format: photocraft_color::PixelFormat,
    intent: photocraft_cms::Intent,
    bpc: bool,
) -> Result<photocraft_raster::Surface> {
    validate_png(&output.png, output.width, output.height)?;
    let mut doc = crate::file_cmds::import("Generated Image.png", &output.png)?;
    crate::color_cmds::convert_document(&mut doc, profile, intent, bpc)?;
    crate::image_cmds::change_depth(&mut doc, format.sample);
    crate::file_cmds::flattened_checked(&doc, format, COMMAND)
}

#[cfg(target_os = "linux")]
fn apply(s: &mut Session, surface: photocraft_raster::Surface, seed: i64, model: &str, background: Option<photocraft_doc::LayerId>) -> Result<Value> {
    let label = if background.is_some() { "Generated Background" } else { "Generated Image" };
    let id = s.edit(label, |doc, active| {
        let name = doc.next_layer_name(label);
        let layer = Layer::new(name, LayerContent::Raster(surface));
        if let Some(subject) = background {
            if doc.layer(subject).is_none() {
                return Err(EngineError::NoLayer(subject));
            }
            Ok(doc.insert_below(Some(subject), layer))
        } else {
            let id = crate::layer_place::insert_new(doc, *active, layer, crate::layer_place::Place::AutoEditable);
            *active = Some(id);
            Ok(id)
        }
    })?;
    Ok(json!({"layer":id.0,"seed":seed,"model":model}))
}

#[cfg(target_os = "linux")]
struct Output {
    png: Vec<u8>,
    width: u32,
    height: u32,
    seed: i64,
    model: String,
}

#[cfg(target_os = "linux")]
fn validate_png(bytes: &[u8], width: u32, height: u32) -> Result<()> {
    if bytes.len() > 5 * 1024 * 1024 || !(64..=1024).contains(&width) || !(64..=1024).contains(&height) {
        return Err(bad("Enki returned an oversized image"));
    }
    let size = bytes.get(16..24).and_then(|b| <[u8; 8]>::try_from(b).ok());
    let expected: Vec<u8> = width.to_be_bytes().into_iter().chain(height.to_be_bytes()).collect();
    if bytes.get(..16) != Some(&b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"[..]) || size.as_ref().map(|v| v.as_slice()) != Some(expected.as_slice()) {
        return Err(bad("Enki returned invalid PNG dimensions"));
    }
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    #[allow(unused_mut)]
    let mut specs = vec![CommandSpec {
        id: COMMAND,
        label: "Generate Image…",
        menu: &["Image"],
        shortcut: None,
        params: r#"{"prompt":text,"negative":text?,"model":id? (empty: Enki recommendation),"width":64..1024=512,"height":64..1024=512 (multiples of 8),"steps":1..50?,"cfg":1..20?,"seed":i64=-1,"document":id?,"placement":"layer|background"="layer","dryRun":bool=false} → {layer,seed,model}. Linux native, installed runtime required. Creates an editable layer in one undo step, or inserts a full-canvas background beneath a masked subject; Session::start runs cancellably in the background."#,
        enabled,
        run,
        journal: true,
    }];
    #[cfg(target_os = "linux")]
    specs.extend(edits::specs());
    #[cfg(target_os = "linux")]
    specs.extend(subject::specs());
    specs
}
