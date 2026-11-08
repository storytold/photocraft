//! File › Open as Smart Object… (#11678): a file opens as a new untitled document with the file's
//! size, resolution, colour mode, bit depth and profile, holding one embedded smart object layer.
//!
//! The layer is built as File › Place Embedded builds one (an embedded smart object at the file's own
//! size and position `0, 0`) and put into the document before it is admitted, so the opening
//! snapshot already shows it.

use super::open::import_opened;
use super::paths::{file_name, read_file, stem, untitled_name};
use super::{Affine, Arc, Document, EngineError, Layer, LayerContent, Result, Session, SmartObject, SmartSource, Value, flattened_checked, json, str_param};
use crate::layer_place::Place;

const CMD: &str = "file.openAsSmartObject";

/// The history step's name.
const LABEL: &str = "Open As Smart Object";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// The files to open: `paths` (several selected files) or `path` (one); whether it was one.
fn paths_param(p: &Value) -> Result<(Vec<String>, bool)> {
    match p.get("paths") {
        None | Some(Value::Null) => Ok((vec![str_param(p, "path", CMD)?.to_string()], true)),
        Some(Value::Array(list)) if p.get("path").is_none_or(Value::is_null) => {
            let paths: Option<Vec<String>> = list.iter().map(|v| v.as_str().filter(|v| !v.is_empty()).map(str::to_string)).collect();
            match paths {
                Some(paths) if !paths.is_empty() => Ok((paths, false)),
                _ => Err(bad("\"paths\" must be a non-empty list of file paths")),
            }
        }
        Some(Value::Array(_)) => Err(bad("give \"path\" or \"paths\", not both")),
        Some(_) => Err(bad("\"paths\" must be a list of file paths")),
    }
}

/// Opens each file as its own document, in order. A file that cannot be read stops the command
/// with an error; the documents opened before it stay open.
pub(super) fn open_as_smart_object(s: &mut Session, p: &Value) -> Result<Value> {
    let (paths, single) = paths_param(p)?;
    let mut opened = Vec::new();
    for path in &paths {
        opened.push(open_one(s, path)?);
    }
    Ok(if single { opened.swap_remove(0) } else { json!({"documents": opened}) })
}

fn open_one(s: &mut Session, path: &str) -> Result<Value> {
    let bytes = read_file(path)?;
    let src = import_opened(s, path, &bytes)?;
    // The file as one image: the smart object's rendered contents.
    let pixels = flattened_checked(&src, src.pixel_format(), CMD)?;
    let layer_name = stem(path);
    // "<name>-1", the next free number while a document with that name is open.
    let name = untitled_name(&layer_name, |n| s.documents().iter().any(|d| d.doc.name == n));

    let mut doc = Document::new(name.clone(), src.size, src.mode, src.depth);
    doc.resolution_dpi = src.resolution_dpi;
    doc.icc_profile = src.icc_profile.clone();
    doc.color_table = src.color_table.clone();
    doc.duotone = src.duotone.clone();

    // Embedded (not linked): the original file name keeps its extension, so Edit Contents opens it.
    let source = SmartSource::Embedded { file_name: file_name(path), bytes: Arc::new(bytes) };
    let so = SmartObject::new(source, Affine { m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] }, Some(pixels));
    let layer = Layer::new(layer_name, LayerContent::Smart(so));
    let layer_id = layer.id;
    crate::layer_place::insert_new(&mut doc, None, layer, Place::AutoEditable);
    let index = admit_as_smart_object(s, doc);
    Ok(json!({"document": index, "name": name, "layer": layer_id.0}))
}

/// Admits a document built by Open as Smart Object (one smart object layer) as an untitled
/// document: the opening snapshot already shows the placed layer, is named after it, and the
/// history is the single state "Open As Smart Object", as File › Open's is one state.
pub fn admit_as_smart_object(s: &mut Session, doc: Document) -> usize {
    let layer_name = doc.layers.first().map(|l| l.name.clone());
    // Untitled: no path, so Save asks where to save.
    let index = s.add_document(doc, None);
    if let Some(st) = s.active_mut() {
        if let (Some(first), Some(name)) = (st.snapshots.iter_mut().find(|snap| snap.id == 0), layer_name) {
            first.name = name;
        }
        st.history.set_current_label(LABEL);
    }
    index
}
