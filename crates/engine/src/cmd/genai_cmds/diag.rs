//! Save Fill Diagnostics (#10903): what a Generative Fill sent and got back, written to a folder
//! the person picks, so a fill that came back wrong can be looked at before prompts or blending
//! are changed. Nothing is logged or written on its own: each fill's capture is kept in memory
//! for the session (the last [`KEEP`] fills, and a failed fill until it is retried or discarded)
//! and leaves the computer's memory only through this command, which automation may not run.
//!
//! The folder holds the area as rendered before the selection was cleared (`crop.png`), the
//! picture, mask and prompt exactly as sent (`source.png`, `mask.png`, `prompt.txt`), any
//! reference pictures as sent (`reference-N.png`), each
//! picture exactly as the service returned it (`returned-N.*`), each placed variation composited
//! into the document as it is now (`composite-N.png`), and `summary.json`: the model and
//! connection, sizes, how each picture was lined up, whether colour matching ran or why it was
//! skipped, and what PhotoCraft can't see (the instruction a service's adapter wraps the prompt
//! in).

use std::collections::VecDeque;
use std::sync::Arc;

use photocraft_doc::{Document, Layer, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use super::{Prepared, VariationNote, pixels};
use crate::genai::Disclosure;
use crate::{EngineError, Result, Session};

pub const SAVE_DIAGNOSTICS: &str = "layer.generative.saveDiagnostics";
/// How many landed fills keep their capture (the oldest is dropped first).
pub(super) const KEEP: usize = 8;

/// Files to write: (name, bytes).
type Files = Vec<(String, Vec<u8>)>;

/// One picture the service returned, as it came, and what became of it.
pub(super) struct Returned {
    pub(super) bytes: Vec<u8>,
    /// How it was placed, or why it wasn't.
    pub(super) outcome: std::result::Result<VariationNote, String>,
    /// The variation layer it became, once it landed.
    pub(super) layer: Option<LayerId>,
}

/// What one fill (or Generate More) sent and got back.
pub(super) struct Capture {
    pub(super) prepared: Arc<Prepared>,
    /// The area sent, in document pixels.
    pub(super) area: Rect,
    /// The prompt as typed (empty: the fixed instruction to continue the picture was sent).
    pub(super) typed: String,
    pub(super) disclosure: Option<Disclosure>,
    pub(super) more: bool,
    pub(super) returned: Vec<Returned>,
    /// Why the fill failed, when it did.
    pub(super) failure: Option<String>,
    /// The generative layer it went into, once it landed.
    pub(super) group: Option<LayerId>,
}

/// The landed fills' captures, oldest first.
#[derive(Default)]
pub(super) struct Captures(VecDeque<Arc<Capture>>);

impl Captures {
    /// Keep `c`, dropping the oldest past [`KEEP`].
    pub(super) fn push(&mut self, c: Capture) {
        self.0.push_back(Arc::new(c));
        while self.0.len() > KEEP {
            self.0.pop_front();
        }
    }

    /// The captures of generative layer `group`, oldest first.
    pub(super) fn of(&self, group: LayerId) -> Vec<Arc<Capture>> {
        self.0.iter().filter(|c| c.group == Some(group)).cloned().collect()
    }
}

impl Session {
    /// Can Save Fill Diagnostics write something for generative layer `layer` (the group or one
    /// of its variations)?
    pub fn generative_has_diagnostics(&self, layer: LayerId) -> bool {
        super::generative_target(self, &json!({"layer": layer.0})).is_some_and(|g| !self.genai_state.captures.of(g).is_empty())
    }
}

/// `fill`, `layer` (or the active generative layer): the captures to write.
fn captures(s: &Session, p: &Value) -> Result<Vec<Arc<Capture>>> {
    let bad = |msg: String| EngineError::BadParams { cmd: SAVE_DIAGNOSTICS.into(), msg };
    if let Some(v) = p.get("fill") {
        let id = v.as_u64().ok_or_else(|| bad("`fill` must be a fill id (see layer.generative.fills)".into()))?;
        let fill = s.genai_state.fills.iter().find(|f| f.id == id).ok_or_else(|| bad(format!("no Generative Fill {id} is waiting")))?;
        return fill.status.capture().map(|c| vec![c]).ok_or_else(|| bad(format!("Generative Fill {id} has nothing to save yet")));
    }
    let group = super::generative_target(s, p).ok_or_else(|| bad("select a generative layer first".into()))?;
    let found = s.genai_state.captures.of(group);
    if found.is_empty() {
        return Err(bad(format!("nothing is kept for this generative layer: only the last {KEEP} fills of this session keep what they sent and got back")));
    }
    Ok(found)
}

/// On for an active generative layer that kept its capture, or while a failed fill keeps one.
pub(super) fn save_enabled(s: &Session) -> std::result::Result<(), String> {
    match s.generative_layer() {
        Some(g) if !s.genai_state.captures.of(g).is_empty() => Ok(()),
        // Whatever layer is active: the failed fill's row offers it (with `fill`).
        _ if s.genai_state.fills.iter().any(|f| f.status.capture().is_some()) => Ok(()),
        Some(_) => Err(format!("nothing is kept for this generative layer: only the last {KEEP} fills of this session keep what they sent and got back")),
        None => Err("select a generative layer first".into()),
    }
}

/// `{"folder", "layer"?, "fill"?}`: write the captures into a new folder inside `folder`.
pub(super) fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: &str| EngineError::BadParams { cmd: SAVE_DIAGNOSTICS.into(), msg: msg.into() };
    let folder = p.get("folder").and_then(Value::as_str).filter(|f| !f.is_empty()).ok_or_else(|| bad("missing `folder` (the app asks for one)"))?;
    let found = captures(s, p)?;
    let doc = s.active().map(|st| Arc::clone(&st.doc)).ok_or(EngineError::NoDocument)?;
    let mut files = Vec::new();
    let mut parts = Vec::new();
    for (i, c) in found.iter().enumerate() {
        let prefix = if found.len() == 1 { String::new() } else { format!("{}-", i + 1) };
        let (written, summary) = capture_files(&doc, c, &prefix)?;
        files.extend(written);
        parts.push(summary);
    }
    let summary = json!({
        "app": "PhotoCraft",
        "what": "What Generative Fill sent and got back, saved because you asked (Save Fill Diagnostics). Nothing of it was logged or written before.",
        "fills": parts,
    });
    let text = serde_json::to_vec_pretty(&summary).map_err(pixels::other)?;
    files.push(("summary.json".to_owned(), text));
    let dir = write_folder(folder, &files)?;
    Ok(json!({"folder": dir, "files": files.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>()}))
}

/// The files for one capture (named with `prefix`) and its part of the summary.
fn capture_files(doc: &Document, c: &Capture, prefix: &str) -> Result<(Files, Value)> {
    let r = &c.prepared.reference;
    let request = &c.prepared.request;
    let crop: Vec<u8> = r
        .original
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]].map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8).into_iter().chain([255]))
        .collect();
    let mut files = vec![
        (format!("{prefix}crop.png"), pixels::encode_rgba(crop, r.w, r.h)?),
        (format!("{prefix}source.png"), request.source_png.clone()),
        (format!("{prefix}mask.png"), request.mask_png.clone()),
        (format!("{prefix}prompt.txt"), request.prompt.as_bytes().to_vec()),
    ];
    files.extend(request.references.iter().enumerate().map(|(k, png)| (format!("{prefix}reference-{}.png", k + 1), png.clone())));
    let mut variations = Vec::new();
    for (k, ret) in c.returned.iter().enumerate() {
        let n = k + 1;
        let ext = match photocraft_codecs::detect(&ret.bytes) {
            Some(photocraft_codecs::Format::Png) => "png",
            Some(photocraft_codecs::Format::Jpeg) => "jpg",
            Some(photocraft_codecs::Format::WebP) => "webp",
            _ => "bin",
        };
        let returned_file = format!("{prefix}returned-{n}.{ext}");
        files.push((returned_file.clone(), ret.bytes.clone()));
        let size = pixels::decode(&ret.bytes).ok().map(|(_, full)| [full.width(), full.height()]);
        let composite = match (&ret.outcome, ret.layer, c.group) {
            (Ok(_), Some(layer), Some(group)) => match composite(doc, group, layer, c.area)? {
                Some(png) => {
                    let name = format!("{prefix}composite-{n}.png");
                    files.push((name.clone(), png));
                    json!(name)
                }
                None => json!(null),
            },
            _ => json!(null),
        };
        variations.push(json!({
            "returned": returned_file,
            "returnedSize": size,
            "placed": ret.outcome.as_ref().ok(),
            "colourMatching": ret.outcome.as_ref().ok().map(|n| if n.colour.skipped() { "skipped" } else { "ran" }),
            "refused": ret.outcome.as_ref().err(),
            "layer": ret.layer.map(|l| l.0),
            "composite": composite,
        }));
    }
    let d = c.disclosure.as_ref();
    let summary = json!({
        "generateMore": c.more,
        "model": d.map(|d| d.model.clone()),
        "connection": d.map(|d| d.connection.clone()),
        "destination": d.map(|d| d.destination.clone()),
        "onThisComputer": d.map(|d| d.local),
        "area": {"x": c.area.x0, "y": c.area.y0, "width": c.area.width(), "height": c.area.height()},
        "requestSize": [r.w, r.h],
        "promptTyped": c.typed,
        "instructionSent": request.prompt,
        "instructionNote": "PhotoCraft sends the prompt above (or, for an empty prompt, its fixed instruction to continue the picture) with the picture and the mask. The shared AI layer's adapter for the service may wrap it in an instruction of its own (Gemini's does); PhotoCraft doesn't see that text.",
        "crop": "the area as rendered from the visible layers in sRGB at the request size, before the selection was cleared (not sent)",
        "composite": "each placed variation shown alone in its generative layer, rendered from the document as it is now over the area sent",
        "variations": variations,
        "failure": c.failure,
    });
    Ok((files, summary))
}

/// The area `area` of `doc` with variation `layer` alone showing in generative layer `group`, as
/// PNG; `None` when either has gone (undone, deleted).
fn composite(doc: &Document, group: LayerId, layer: LayerId, area: Rect) -> Result<Option<Vec<u8>>> {
    let mut doc = doc.clone();
    let Some(children) = doc.layer_mut(group).and_then(Layer::children_mut) else { return Ok(None) };
    if !children.iter().any(|l| l.id == layer) {
        return Ok(None);
    }
    for l in children.iter_mut() {
        l.visible = l.id == layer;
    }
    if let Some(g) = doc.layer_mut(group) {
        g.visible = true;
    }
    let image = pixels::srgb_composite(&doc, area)?;
    let rgba = image.convert(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, true)).to_interleaved(area);
    pixels::encode_rgba(rgba, area.width(), area.height()).map(Some)
}

/// Write `files` into a new folder inside `folder`, named so nothing there is replaced.
#[cfg(not(target_arch = "wasm32"))]
fn write_folder(folder: &str, files: &[(String, Vec<u8>)]) -> Result<String> {
    let base = std::path::Path::new(folder);
    if !base.is_dir() {
        return Err(EngineError::Other(format!("{folder} is not a folder")));
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let stamp = i64::try_from(secs)
        .ok()
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map_or_else(|| secs.to_string(), |t| t.format("%Y-%m-%d %H.%M.%S UTC").to_string());
    let dir = (1..100)
        .map(|n| base.join(if n == 1 { format!("Generative Fill diagnostics {stamp}") } else { format!("Generative Fill diagnostics {stamp} ({n})") }))
        .find(|d| !d.exists())
        .ok_or_else(|| EngineError::Other(format!("{folder} already holds diagnostics from this second")))?;
    std::fs::create_dir(&dir).map_err(|e| EngineError::Other(format!("{}: {e}", dir.display())))?;
    for (name, bytes) in files {
        if let Err(e) = crate::file_cmds::write_file(&dir.join(name).to_string_lossy(), bytes) {
            // This call made `dir`: take it away rather than leave half the files in it.
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    }
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(target_arch = "wasm32")]
fn write_folder(folder: &str, _: &[(String, Vec<u8>)]) -> Result<String> {
    Err(EngineError::Other(format!("cannot write to {folder}: Save Fill Diagnostics needs the desktop app")))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn a_failed_write_leaves_no_folder_behind() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![("first.txt".to_owned(), b"one".to_vec()), ("sec\0ond.txt".to_owned(), b"two".to_vec())];
        assert!(super::write_folder(&dir.path().to_string_lossy(), &files).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "the half-written folder is taken away");
        let ok = super::write_folder(&dir.path().to_string_lossy(), &files[..1]).unwrap();
        assert!(std::path::Path::new(&ok).join("first.txt").is_file());
    }
}
