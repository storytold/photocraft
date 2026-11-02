//! Reference images for `generate.edit` and `generate.image` (ShotoPhopEX's reference deck).
//!
//! A command's `references` param is a list of specs, each one of
//!
//! - `{"layer": id}`: a pixel layer of the active document, its pixels over its content bounds
//!   (transparent areas over white, since ComfyUI's loader drops alpha);
//! - `{"path": "…"}`: an image file (native only; any format `photocraft-codecs` decodes);
//! - `{"job": id, "index": i}`: a result stored in the Generations library.
//!
//! Each becomes straight RGBA8, scaled to at most [`MAX_REFERENCE_PIXELS`] (the encoders resize
//! to about a megapixel themselves, so a larger upload only costs time). How many a template
//! takes is its `references.max` (and `min`); the caller checks that against the template it
//! runs.

use photocraft_doc::LayerId;
use photocraft_genai::Rgba8;
use photocraft_geom::Rect;
use serde_json::Value;

use crate::generate_cmds::{bad, gen_err, resize_rgba8};
use crate::{EngineError, Result, Session};

/// Most references any request may carry (the largest template's slots).
pub const MAX_REFERENCES: usize = 9;
/// References are scaled down to this many pixels before upload.
pub const MAX_REFERENCE_PIXELS: u64 = 1024 * 1024;
/// A reference source larger than this is refused rather than read.
const MAX_SOURCE_PIXELS: u64 = 64 * 1024 * 1024;
/// A reference file larger than this is refused rather than read.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// A request's references: the specs as given (remembered on the result layer so Generate
/// Similar resolves them again) and their pixels.
#[derive(Clone, Debug, Default)]
pub(crate) struct Refs {
    pub(crate) specs: Vec<Value>,
    pub(crate) images: Vec<Rgba8>,
}

/// Parse, check against `template` and resolve the `references` of `p`.
pub(crate) fn plan(s: &Session, cmd: &str, p: &Value, template: &photocraft_genai::template::Template) -> Result<Refs> {
    let specs = specs(cmd, p)?;
    check_count(cmd, template, specs.len())?;
    let images = resolve(s, &specs)?;
    Ok(Refs { specs, images })
}

/// The `references` specs of `p` (validated, not resolved): `[]` when absent.
pub(crate) fn specs(cmd: &str, p: &Value) -> Result<Vec<Value>> {
    let list = match p.get("references") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(bad(cmd, "`references` must be a list of {\"layer\": id}, {\"path\": \"…\"} or {\"job\": id, \"index\": i}")),
    };
    if list.len() > MAX_REFERENCES {
        return Err(bad(cmd, format!("at most {MAX_REFERENCES} references (got {})", list.len())));
    }
    for (i, r) in list.iter().enumerate() {
        let o = r.as_object().ok_or_else(|| bad(cmd, format!("reference {} must be an object", i + 1)))?;
        let kinds = ["layer", "path", "job"].iter().filter(|k| o.contains_key(**k)).count();
        if kinds != 1 {
            return Err(bad(cmd, format!("reference {} needs exactly one of `layer`, `path` or `job`", i + 1)));
        }
        if let Some(v) = o.get("layer")
            && v.as_u64().is_none()
        {
            return Err(bad(cmd, format!("reference {}: `layer` must be a layer id", i + 1)));
        }
        if let Some(v) = o.get("path")
            && v.as_str().is_none_or(|s| s.trim().is_empty() || s.len() > 4096)
        {
            return Err(bad(cmd, format!("reference {}: `path` must be a file path", i + 1)));
        }
        if let Some(v) = o.get("job") {
            if v.as_u64().is_none() {
                return Err(bad(cmd, format!("reference {}: `job` must be a generation id", i + 1)));
            }
            if o.get("index").is_some_and(|x| x.as_u64().is_none_or(|n| n > 63)) {
                return Err(bad(cmd, format!("reference {}: `index` must be 0..63", i + 1)));
            }
        }
    }
    Ok(list.clone())
}

/// Resolve validated specs to pixels (on the UI thread: reading a layer needs the document).
pub(crate) fn resolve(s: &Session, specs: &[Value]) -> Result<Vec<Rgba8>> {
    specs.iter().enumerate().map(|(i, r)| one(s, r).map_err(|e| EngineError::Other(format!("reference {}: {e}", i + 1)))).collect()
}

fn one(s: &Session, r: &Value) -> Result<Rgba8> {
    let img = if let Some(id) = r.get("layer").and_then(Value::as_u64) {
        layer(s, LayerId(id))?
    } else if let Some(path) = r.get("path").and_then(Value::as_str) {
        file(path)?
    } else if let Some(job) = r.get("job").and_then(Value::as_u64) {
        let index = r.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let path = s.generations.result_path(job, index).ok_or_else(|| EngineError::Other(format!("generation {job} has no stored result {index}")))?;
        let bytes = std::fs::read(&path).map_err(|e| EngineError::Other(format!("the stored result is missing ({e})")))?;
        photocraft_genai::png::decode_rgba8(&bytes).map_err(gen_err)?
    } else {
        return Err(EngineError::Other("no `layer`, `path` or `job`".into()));
    };
    fit(img)
}

/// Scale `img` down to [`MAX_REFERENCE_PIXELS`] (aspect kept).
fn fit(img: Rgba8) -> Result<Rgba8> {
    let (w, h) = crate::generate_cmds::fit_pixels(img.width, img.height, MAX_REFERENCE_PIXELS);
    if (w, h) == (img.width, img.height) { Ok(img) } else { resize_rgba8(&img, w, h) }
}

fn layer(s: &Session, id: LayerId) -> Result<Rgba8> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let layer = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let surf = layer.surface().ok_or_else(|| EngineError::Other(format!("“{}” is not a pixel layer; rasterize it or use a file", layer.name)))?;
    let rect: Rect = surf.content_bounds();
    if rect.is_empty() {
        return Err(EngineError::Other(format!("“{}” is empty", layer.name)));
    }
    if u64::from(rect.width()) * u64::from(rect.height()) > MAX_SOURCE_PIXELS {
        return Err(EngineError::Other(format!("“{}” is larger than 64 megapixels", layer.name)));
    }
    let mut px = vec![[0u8; 4]; rect.width() as usize * rect.height() as usize];
    surf.read_rgba8_into(rect, &mut px);
    let data = px
        .into_iter()
        .flat_map(|p| {
            let a = u16::from(p[3]);
            let over = |c: u8| ((u16::from(c) * a + 255 * (255 - a)) / 255) as u8;
            [over(p[0]), over(p[1]), over(p[2]), 255]
        })
        .collect();
    Rgba8::new(rect.width(), rect.height(), data).map_err(gen_err)
}

#[cfg(not(target_arch = "wasm32"))]
fn file(path: &str) -> Result<Rgba8> {
    let meta = std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(EngineError::Other(format!("{path} is larger than 256 MB")));
    }
    let bytes = std::fs::read(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let img = photocraft_codecs::decode(&bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    if u64::from(img.width()) * u64::from(img.height()) > MAX_SOURCE_PIXELS {
        return Err(EngineError::Other(format!("{path} is larger than 64 megapixels")));
    }
    let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
    let (w, h) = (rgba.width(), rgba.height());
    Rgba8::new(w, h, rgba.data().to_vec()).map_err(gen_err)
}

#[cfg(target_arch = "wasm32")]
fn file(_: &str) -> Result<Rgba8> {
    Err(EngineError::Other("file references are not available on the web; add the image as a layer".into()))
}

/// Check `n` references against a template's limits.
pub(crate) fn check_count(cmd: &str, template: &photocraft_genai::template::Template, n: usize) -> Result<()> {
    let r = &template.meta.references;
    if n > r.max as usize {
        return Err(bad(
            cmd,
            if r.max == 0 {
                format!("template `{}` takes no reference images", template.meta.id)
            } else {
                format!("template `{}` takes at most {} reference image(s) (got {n})", template.meta.id, r.max)
            },
        ));
    }
    if n < r.min as usize {
        return Err(bad(cmd, format!("template `{}` needs at least {} reference image(s)", template.meta.id, r.min)));
    }
    Ok(())
}

#[cfg(test)]
#[path = "generate_refs_tests.rs"]
mod tests;
