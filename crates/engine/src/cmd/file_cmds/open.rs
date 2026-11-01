use super::paths::{file_name, read_file, stem, template_name};
use super::{Arc, Document, EngineError, Layer, LayerContent, PixelFormat, PlaceSteps, Result, Session, Value, json, str_param};

pub(crate) fn import(name: &str, bytes: &[u8]) -> Result<Document> {
    import_result(name, bytes).map(|r| r.document)
}

/// [`import`] for a file that becomes a document of its own (Revert, Load Files into Stack;
/// Batch's scratch sessions use [`import_at_raw_ppi`]): a developed raw gets Preferences › Raw
/// Defaults › Resolution, as it does through File › Open.
pub(crate) fn import_opened(s: &Session, name: &str, bytes: &[u8]) -> Result<Document> {
    import_at_raw_ppi(s.prefs().raw_defaults.resolution, name, bytes)
}

pub(super) fn import_at_raw_ppi(raw_ppi: f64, name: &str, bytes: &[u8]) -> Result<Document> {
    let r = import_result(name, bytes)?;
    let mut doc = r.document;
    raw_resolution(raw_ppi, &r.warnings, &mut doc);
    Ok(doc)
}

fn import_result(name: &str, bytes: &[u8]) -> Result<photocraft_io::ImportResult> {
    crate::allocation::checkpoint("importing document")?;
    let r = photocraft_io::import(name, bytes).map_err(|e| EngineError::Other(format!("{name}: {e}")))?;
    // Auxiliary imports return only a document and cannot surface the preview's fidelity warning.
    // Open has its own warning-preserving path; never silently place or process a thumbnail.
    if r.preview_only {
        return Err(EngineError::Other(format!(
            "{name}: only this Affinity file's embedded preview could be read; open it with File › Open to see the warning, or export PSD or PNG from Affinity before using it here"
        )));
    }
    Ok(r)
}

// ---------- close / revert / save a copy / open as ----------

pub(super) fn close_all(s: &mut Session) -> Result<Value> {
    let n = s.documents().len();
    while !s.documents().is_empty() {
        s.close(s.documents().len() - 1);
    }
    Ok(json!({"closed": n}))
}

pub(super) fn close_others(s: &mut Session, p: &Value) -> Result<Value> {
    let keep = p.get("document").and_then(Value::as_u64).map(|v| v as usize).or(s.active_index()).ok_or(EngineError::NoDocument)?;
    if keep >= s.documents().len() {
        return Err(EngineError::BadParams { cmd: "file.closeOthers".into(), msg: format!("no document {keep}") });
    }
    let n = s.documents().len() - 1;
    for i in (0..s.documents().len()).rev() {
        if i != keep {
            s.close(i);
        }
    }
    s.set_active(0);
    Ok(json!({"closed": n}))
}

/// File › Revert: reload the saved file as one history step (undoable, like Photoshop's
/// "Revert" history state); the document is clean afterwards.
pub(super) fn revert(s: &mut Session) -> Result<Value> {
    let path = s.active().and_then(|d| d.path.clone()).ok_or(EngineError::Other("the document has never been saved".into()))?;
    let bytes = read_file(&path)?;
    let fresh = import_opened(s, &file_name(&path), &bytes)?;
    s.edit("Revert", |doc, active| {
        let (id, name) = (doc.id, doc.name.clone());
        *doc = fresh;
        // Keep the identity (colour state, views and caches are keyed by it) and the tab name.
        doc.id = id;
        doc.name = name;
        *active = doc.top_layer();
        Ok(())
    })?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.saved_revision = st.revision;
    Ok(json!({"path": path, "layers": st.doc.layers.len()}))
}

/// Open a file's bytes, decoding them as the format `as_ext` (Open As) when given.
/// EXR/HDR rarely states its colour space, so io tags it with the linear-sRGB default.
/// With File Handling's ACEScg option on, such an EXR opens as ACEScg instead (AP1,
/// linear) — the working space ACES render pipelines write. Only the io default is
/// replaced; an embedded or EXIF-declared profile is kept untouched.
pub(crate) fn tag_exr_colorspace(s: &Session, name: &str, doc: &mut Document) {
    if !name.to_ascii_lowercase().ends_with(".exr") || !s.prefs.get().file_handling.exr_aces_cg {
        return;
    }
    let linear = photocraft_cms::Builtin::LinearSrgb.profile().to_bytes();
    if doc.icc_profile.as_ref().map(|v| v.as_slice()) == Some(linear.as_slice()) {
        doc.icc_profile = Some(photocraft_cms::Builtin::AcesCg.profile().to_bytes());
    }
}

/// Edit › Preferences › Raw Defaults › Resolution: the ppi a raw developed on open gets, as
/// Camera Raw's workflow options set it (the raw itself states none). `warnings` are the
/// import's notes, which say whether the raw was developed or only its preview opened.
pub fn raw_defaults(s: &Session, warnings: &[String], doc: &mut Document) {
    raw_resolution(s.prefs().raw_defaults.resolution, warnings, doc);
}

fn raw_resolution(ppi: f64, warnings: &[String], doc: &mut Document) {
    if ppi.is_finite() && warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)) {
        doc.resolution_dpi = ppi.clamp(1.0, 10_000.0) as f32;
    }
}

pub fn open_bytes_as(s: &mut Session, name: &str, bytes: &[u8], as_ext: Option<&str>, path: Option<String>) -> Result<Value> {
    crate::allocation::checkpoint("importing document")?;
    let decode_name = match as_ext {
        Some(ext) => format!("{}.{}", stem(name), ext.trim_start_matches('.')),
        None => name.to_string(),
    };
    let r = photocraft_io::import(&decode_name, bytes).map_err(|e| EngineError::Other(format!("{decode_name}: {e}")))?;
    let mut doc = r.document;
    tag_exr_colorspace(s, &decode_name, &mut doc);
    raw_defaults(s, &r.warnings, &mut doc);
    let path = match template_name(s, name) {
        Some(untitled) => {
            doc.name = untitled;
            None
        }
        None => {
            doc.name = file_name(name);
            path
        }
    };
    // Color Settings policies (preserve / convert / discard the embedded profile).
    let (i, color) = s.open_document(doc, path.filter(|_| !r.source_read_only));
    if let Some(st) = s.active_mut() {
        st.source_read_only = r.source_read_only;
    }
    // Import notes (e.g. how a camera raw was developed, or that only its preview opened).
    Ok(json!({"document": i, "color": color, "warnings": r.warnings}))
}

pub(super) fn open_as(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path", "file.openAs")?.to_string();
    let bytes = read_file(&path)?;
    let as_ext = p.get("as").or_else(|| p.get("format")).and_then(Value::as_str);
    open_bytes_as(s, &path, &bytes, as_ext, Some(path.clone()))
}

// ---------- place ----------

/// History label of an embedded place.
pub const PLACE_EMBEDDED: &str = "Place Embedded";

/// Place a file's bytes as a smart object layer, centred, and (with Preferences ▸ General ▸
/// Resize Image During Place, on by default) scaled down to fit when larger than the canvas.
/// `linked` makes it a linked smart object that refers to that path instead of embedding the bytes.
/// With Preferences ▸ General ▸ Always Create Smart Objects When Placing off, an embedded place
/// lands as a plain pixel layer instead (an explicit `"smartObject": false` asks for one).
///
/// The decoding and resampling are a background job when the command was started with
/// [`Session::start`] (see [`crate::jobs`]): on a worker thread, or where there are none (the web
/// build) a slice per frame on the calling thread ([`PlaceSteps`]); inline otherwise. The layer is
/// inserted by the apply step on the calling thread, as one undo step.
pub fn place_bytes(s: &mut Session, name: &str, bytes: Vec<u8>, linked: Option<String>, p: &Value) -> Result<Value> {
    let input = PlaceInput::new(s, name, bytes, linked, p)?;
    let label = input.job_label();
    crate::jobs::run_stepped(s, &label, true, PlaceSteps::new(input, s.cooperative_jobs()), apply_place)
}

/// Everything a place reads from the session, taken up front so [`PlaceSteps`] can run on a
/// worker that never sees the [`Session`].
pub(crate) struct PlaceInput {
    pub(super) name: String,
    /// Shared with the decoder and the embedded source, so the bytes exist once.
    pub(super) bytes: Arc<Vec<u8>>,
    pub(super) linked: Option<String>,
    pub(super) params: Value,
    pub(super) canvas: (f64, f64),
    pub(super) fmt: PixelFormat,
    pub(super) opts: photocraft_io::ImportOptions,
    /// Preferences ▸ General ▸ Resize Image During Place, unless the params say otherwise.
    pub(super) fit: bool,
    /// Preferences ▸ General ▸ Always Create Smart Objects When Placing, unless the params say
    /// otherwise. A linked place is always a smart object.
    pub(super) smart: bool,
    /// The document as the job sees it, for rendering a vector source at its placement scale.
    pub(super) doc: Arc<Document>,
}

impl PlaceInput {
    pub(crate) fn new(s: &Session, name: &str, bytes: Vec<u8>, linked: Option<String>, p: &Value) -> Result<Self> {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        let prefs = s.prefs();
        Ok(Self {
            name: name.to_string(),
            canvas: (d.doc.size.width as f64, d.doc.size.height as f64),
            fmt: d.doc.pixel_format(),
            opts: crate::jobs::import_options(prefs),
            // The implicit `fit` is Preferences ▸ General ▸ Resize Image During Place (on by default).
            fit: p.get("fit").and_then(Value::as_bool).unwrap_or(prefs.general.resize_image_during_place),
            // Preferences ▸ General ▸ Always Create Smart Objects When Placing (on by default): off,
            // an embedded place lands as a plain pixel layer.
            smart: linked.is_some() || p.get("smartObject").and_then(Value::as_bool).unwrap_or(prefs.general.always_create_smart_objects_when_placing),
            doc: Arc::clone(&d.doc),
            params: p.clone(),
            bytes: Arc::new(bytes),
            linked,
        })
    }

    /// The job's name in the progress UI.
    pub(crate) fn job_label(&self) -> String {
        format!("Placing {}", file_name(&self.name))
    }

    /// The command this place stands for.
    pub(crate) fn command(&self) -> &'static str {
        if self.linked.is_some() { "file.placeLinked" } else { "file.placeEmbedded" }
    }
}

/// A place decoded, flattened and fitted, ready to be inserted by [`apply_place`].
pub(crate) struct Prepared {
    pub(super) content: LayerContent,
    pub(super) layer_name: String,
    /// The history step's name.
    pub(super) label: &'static str,
    pub(super) scale: f64,
    pub(super) bounds: [f64; 4],
    pub(super) smart: bool,
}

/// Insert the prepared layer as one undo step and report the place. Runs on the session's thread.
pub(crate) fn apply_place(s: &mut Session, p: Prepared) -> Result<Value> {
    let Prepared { content, layer_name, label, scale, bounds, smart } = p;
    let id = s.edit(label, move |doc, active| {
        let id = crate::layer_place::insert_new(doc, *active, Layer::new(layer_name, content), crate::layer_place::Place::AutoEditable);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "scale": scale * 100.0, "bounds": bounds, "smartObject": smart}))
}

pub(super) fn place(s: &mut Session, p: &Value, linked: bool) -> Result<Value> {
    let cmd = if linked { "file.placeLinked" } else { "file.placeEmbedded" };
    let path = str_param(p, "path", cmd)?.to_string();
    let bytes = read_file(&path)?;
    // The link is stored absolute: a relative one would later be taken from the document's
    // folder (`doc_path`), not the working directory the path was given in.
    let link = linked.then(|| {
        if std::path::Path::new(&path).is_relative() {
            std::path::absolute(&path).map_or_else(|_| path.clone(), |a| a.to_string_lossy().into_owned())
        } else {
            path.clone()
        }
    });
    place_bytes(s, &path, bytes, link, p)
}
