//! File › Open as Smart Object… (#11678): a file opens as a new untitled document with the file's
//! size, resolution, colour mode, bit depth and profile, holding one embedded smart object layer.
//!
//! The layer is built as File › Place Embedded builds one (an embedded smart object at the file's own
//! size and position `0, 0`) and put into the document before it is admitted, so the opening
//! snapshot already shows it.

use super::open::import_result_for_open;
use super::paths::{file_name, read_file, stem, untitled_name};
use super::{Affine, Arc, Document, Layer, LayerContent, Result, Session, SmartObject, SmartSource, Value, flattened_checked, json, str_param};
use crate::jobs::{JobCtx, Started};
use crate::layer_place::Place;
use photocraft_io::{HistoricalXcfSampleOrderRequest, ImportOptions, ImportResult, IoError};
use photocraft_raster::Surface;

const CMD: &str = "file.openAsSmartObject";

/// The history step's name.
const LABEL: &str = "Open As Smart Object";

struct SourcePolicy {
    damaged: bool,
    historical_xcf_sample_order: Option<photocraft_doc::HistoricalXcfSampleOrder>,
}

/// Opens the file as a document holding one embedded smart object. The read, decode and flatten
/// run on the job's worker (inline under `execute`); the document is built, given File › Open's
/// colour policy and admitted, or held for its profile prompt, when the job is applied. The reply
/// has the shape of File › Open's job: `{document, docId, name, layer, warnings, color}`, or
/// `{held, name, warnings, color, smartObject}` while the profile prompt waits.
pub(super) fn open_as_smart_object(s: &mut Session, p: &Value) -> Result<Value> {
    let opts = super::open::import_options_from_params(crate::jobs::import_options(s.prefs()), p, CMD)?;
    let path = str_param(p, "path", CMD)?.to_string();
    let name = file_name(&path);
    // An interactive caller (the app) asks before opening a PSD that had damaged data, as File > Open does.
    let confirm_damage = p.get("confirmDamage").and_then(Value::as_bool).unwrap_or(false);
    let ask_order = p.get("askHistoricalXcfSampleOrder").and_then(Value::as_bool).unwrap_or(false) && opts.historical_xcf_sample_order.is_none();
    crate::jobs::run(
        s,
        &format!("Opening {name}"),
        false,
        move |ctx| {
            ctx.progress(0.0, "Reading");
            let bytes = Arc::new(read_file(&path)?);
            if ask_order {
                let question = ctx
                    .stage(0., 0.05, "Reading", |ctl| photocraft_io::historical_xcf_sample_order_request_with(&name, &bytes, ctl))
                    .map_err(|error| source_error(&name, error))?;
                if let Some(question) = question {
                    return Ok(SmartOpen::Historical { path, bytes, question, options: opts });
                }
            }
            prepare(path, bytes, opts, confirm_damage, ctx).map(|ready| SmartOpen::Ready(Box::new(ready)))
        },
        |s, result| match result {
            SmartOpen::Historical { path, bytes, question, options } => crate::jobs::hold_smart_object(s, path, bytes, question, options),
            SmartOpen::Ready(ready) => ready.apply(s),
        },
    )
}

enum SmartOpen {
    Historical { path: String, bytes: Arc<Vec<u8>>, question: HistoricalXcfSampleOrderRequest, options: ImportOptions },
    Ready(Box<Ready>),
}

struct Ready {
    path: String,
    file: String,
    bytes: Arc<Vec<u8>>,
    result: ImportResult,
    pixels: Surface,
    policy: SourcePolicy,
}

impl Ready {
    fn apply(self, s: &mut Session) -> Result<Value> {
        apply(s, &self.path, &self.file, self.bytes, self.result, self.pixels, self.policy)
    }
}

fn source_error(name: &str, error: IoError) -> crate::EngineError {
    match error {
        IoError::Cancelled => crate::EngineError::Cancelled,
        error => crate::error::io_open_error(name, error),
    }
}

fn prepare(path: String, bytes: Arc<Vec<u8>>, options: ImportOptions, confirm_damage: bool, ctx: &JobCtx) -> Result<Ready> {
    ctx.check()?;
    let file = file_name(&path);
    let (result, damaged) = ctx.stage(0.05, 0.9, "Decoding", |ctl| import_result_for_open(&file, &bytes, &options, ctl))?;
    let historical_xcf_sample_order = ctx
        .stage(0.9, 0.91, "Flattening", |ctl| photocraft_io::applicable_historical_xcf_sample_order(&file, &bytes, options.historical_xcf_sample_order, ctl))
        .map_err(|error| source_error(&file, error))?;
    ctx.check()?;
    ctx.progress(0.91, "Flattening");
    let pixels = flattened_checked(&result.document, result.document.pixel_format(), CMD)?;
    let policy = SourcePolicy { damaged: confirm_damage && damaged, historical_xcf_sample_order };
    Ok(Ready { path, file, bytes, result, pixels, policy })
}

/// Decode the exact source held by the interactive sample-order prompt.
pub(crate) fn resume_smart_object(s: &mut Session, path: String, bytes: Arc<Vec<u8>>, options: ImportOptions, background: bool) -> Result<Started> {
    let name = file_name(&path);
    s.start_job_with_spawn(
        CMD,
        json!({"path":path,"historicalXcfSampleOrder":options.historical_xcf_sample_order}),
        &format!("Opening {name}"),
        false,
        background,
        move |ctx| prepare(path, bytes, options, false, ctx),
        |s, ready| ready.apply(s),
    )
}

fn apply(
    s: &mut Session,
    path: &str,
    file: &str,
    bytes: Arc<Vec<u8>>,
    r: photocraft_io::ImportResult,
    pixels: photocraft_raster::Surface,
    policy: SourcePolicy,
) -> Result<Value> {
    let SourcePolicy { damaged, historical_xcf_sample_order } = policy;
    let warnings = r.warnings;
    let mut src = r.document;
    super::tag_exr_colorspace(s, file, &mut src);
    super::raw_defaults(s, &warnings, &mut src);
    let layer_name = stem(path);
    // "<name>-1", the next free number while a document with that name is open.
    let name = untitled_name(&layer_name, |n| s.documents().iter().any(|d| d.doc.name == n) || s.held_open_named(n));

    let mut doc = Document::new(name.clone(), src.size, src.mode, src.depth);
    doc.resolution_dpi = src.resolution_dpi;
    doc.icc_profile = src.icc_profile.clone();
    doc.color_table = src.color_table.clone();
    doc.duotone = src.duotone.clone();

    // Embedded (not linked): the original file name keeps its extension, so Edit Contents opens it.
    let source = SmartSource::Embedded { file_name: file.to_string(), bytes };
    let mut so = SmartObject::new(source, Affine { m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] }, Some(pixels));
    so.historical_xcf_sample_order = historical_xcf_sample_order;
    let layer = Layer::new(layer_name, LayerContent::Smart(so));
    let layer_id = layer.id;
    crate::layer_place::insert_new(&mut doc, None, layer, Place::AutoEditable);

    // File › Open's completion: a raw has no embedded profile to ask about; otherwise the profile
    // prompt holds the document, and without a question the Color Settings policy is applied.
    let raw = warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE));
    let question = s.color.open_question(&doc).filter(|_| !raw);
    if damaged || question.is_some() {
        let held = s.hold_open(doc, false);
        return Ok(json!({"held": held, "name": name, "warnings": warnings, "color": question, "damaged": damaged, "smartObject": true}));
    }
    let color = s.color.open_policy(&mut doc, s.prefs().file_handling.ignore_exif_profile_tag);
    let index = admit_as_smart_object(s, doc);
    let id = s.documents().get(index).map(|st| st.doc.id.0);
    // Admission numbers the document again, so the reply reads the name it was admitted under.
    let name = s.documents().get(index).map_or(name, |st| st.doc.name.clone());
    Ok(json!({"document": index, "docId": id, "name": name, "layer": layer_id.0, "warnings": warnings, "color": color, "smartObject": true}))
}

/// Admits a document built by Open as Smart Object (one smart object layer) as an untitled
/// document: the opening snapshot already shows the placed layer, is named after it, and the
/// history is the single state "Open As Smart Object", as File › Open's is one state.
pub fn admit_as_smart_object(s: &mut Session, mut doc: Document) -> usize {
    let layer_name = doc.layers.first().map(|l| l.name.clone());
    // The number was free when the file was read, but documents held for their prompts did not count
    // then, and one of them may have been cancelled since: take the first free one now, as each
    // document is admitted.
    if let Some(base) = &layer_name {
        doc.name = untitled_name(base, |n| s.documents().iter().any(|d| d.doc.name == n));
    }
    // Untitled: no path, so Save asks where to save.
    let index = s.add_document(doc, None);
    if let Some(st) = s.active_mut() {
        if let (Some(first), Some(name)) = (st.snapshots_mut().iter_mut().find(|snap| snap.id == 0), layer_name) {
            first.name = name;
        }
        st.history.set_current_label(LABEL);
    }
    index
}
