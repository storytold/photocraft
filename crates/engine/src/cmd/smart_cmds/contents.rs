use super::filters::{smart, smart_mut};
use super::placing::{detach_psd, snap_affine};
use super::rendering::{refresh, refresh_or_fail, source_box, unapplied_filters};
use super::source::{authorized_source_bytes, source_bytes};
use super::{
    Affine, Arc, DocId, Document, EngineError, LayerContent, LayerId, Metadata, Result, Session, SmartContentsId, SmartLink, SmartObject, SmartSource, Value,
    affine_homography, bad, base_name, decode_source, encode_source, json, layer_param, other, placement, set_placement,
};

pub(super) fn path_param<'a>(cmd: &str, p: &'a Value) -> Result<&'a str> {
    p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "pass `path`"))
}

/// Instances sharing contents in this document, including those inside groups.
fn instances(doc: &Document, contents_id: SmartContentsId) -> Vec<LayerId> {
    doc.walk().into_iter().filter_map(|(_, _, l)| matches!(&l.content, LayerContent::Smart(sm) if sm.contents_id == contents_id).then_some(l.id)).collect()
}

/// Swaps shared contents, keeping each instance's transform and filters, and re-renders.
/// `fit` scales the new contents into the frame they replace (as Photoshop's Replace Contents
/// does); the commands that only re-point at the *same* artwork pass `false`.
pub(super) fn set_source(
    s: &mut Session,
    p: &Value,
    label: &str,
    keep_psd: bool,
    make: impl FnOnce(&Metadata, &SmartSource) -> Result<SmartSource>,
) -> Result<Value> {
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let new = make(&doc.metadata, &smart(doc, id)?.source)?;
    set_shared_source(s, id, label, keep_psd, new, false)
}

// Keep the shared-instance transaction out of the per-command source-builder instantiations.
fn set_shared_source(s: &mut Session, id: LayerId, label: &str, keep_psd: bool, new: SmartSource, fit: bool) -> Result<Value> {
    let (parent, contents_id, changed, warnings) = s.edit(label, |doc, _| {
        let contents_id = smart(doc, id)?.contents_id;
        let changed = instances(doc, contents_id);
        let mut warnings = std::collections::BTreeSet::new();
        for &instance in &changed {
            // The frame the replaced contents filled, measured while they are still the source.
            let frame = if fit { contents_frame(doc, smart(doc, instance)?) } else { None };
            smart_mut(doc, instance)?.source = new.clone();
            if !keep_psd {
                detach_psd(doc.layer_mut(instance).ok_or(EngineError::NoLayer(instance))?);
            }
            if fit {
                warnings.extend(fit_replaced_contents(doc, instance, frame)?);
            }
            warnings
                .extend(unapplied_filters(smart(doc, instance)?).into_iter().map(|n| format!("the {n} smart filter is not supported, so it was not applied")));
            refresh_or_fail(doc, instance)?;
        }
        Ok((doc.id, contents_id, changed, warnings))
    })?;
    // An old editor must not overwrite newly replaced/relinked contents on close. Keep its
    // document open, but the next Edit Contents opens the replacement source.
    s.smart_links.retain(|l| l.parent != parent || l.contents_id != contents_id);
    // Every instance changed, not only `layer`: list them, so a caller doesn't repeat the
    // command once per copy (#4075).
    Ok(json!({"layer": id.0, "instances": changed.iter().map(|l| l.0).collect::<Vec<_>>(), "warnings": warnings}))
}

/// How much of the frame a fitted replacement leaves empty before the caller is told about it:
/// a tenth of a percent of the frame's own width or height.
const FRAME_SLACK: f64 = 1e-3;

/// The contents' box in source pixels: what the placement maps into the document, so the box of
/// the art as it stands. Contents PhotoCraft can't decode itself (a native AI, PDF or EPS vector
/// with no display preview, #11529) keep the frame their PSD placed layer recorded.
fn contents_frame(doc: &Document, sm: &SmartObject) -> Option<[f64; 4]> {
    source_box(doc, sm).or_else(|| placed_frame(sm))
}

/// The source-pixel frame a PSD placed layer records for its embedded file.
fn placed_frame(sm: &SmartObject) -> Option<[f64; 4]> {
    let placed = photocraft_io::smart_map::parse_sold(sm.psd_raw.as_ref()?.as_slice())?;
    let (w, h) = photocraft_io::smart_map::stored_size(&placed.descriptor)?;
    (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some([0.0, 0.0, w, h])
}

/// The map in source pixels that fits `new`'s box into `old`'s: uniform, so the art is never
/// distorted (a different shape is reported instead), and centred, so the frame keeps its middle.
/// `None` when a box is degenerate, so the caller keeps the placement and says so.
fn fit_map(old: [f64; 4], new: [f64; 4]) -> Option<Affine> {
    let [x0, y0, x1, y1] = old;
    let [u0, v0, u1, v1] = new;
    let (ow, oh, nw, nh) = (x1 - x0, y1 - y0, u1 - u0, v1 - v0);
    if [ow, oh, nw, nh].iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return None;
    }
    let k = (ow / nw).min(oh / nh);
    if !k.is_finite() || k <= 0.0 {
        return None;
    }
    Some(Affine { m: [k, 0.0, 0.0, k, (x0 + x1) / 2.0 - k * (u0 + u1) / 2.0, (y0 + y1) / 2.0 - k * (v0 + v1) / 2.0] })
}

/// Whether the fit changes the placement at all: the new contents already sit in the frame, so
/// Replace Contents stays silent (and the transform stays exactly as it was, lossless).
fn already_fitted(m: &Affine) -> bool {
    let [a, b, c, d, e, f] = m.m;
    [(a - 1.0), b, c, (d - 1.0), e, f].iter().all(|v| v.abs() < 1e-9)
}

/// Scales smart object `id`'s new contents into the frame its old contents filled, keeping the
/// frame's position, rotation and size ([`fit_map`]): Photoshop's Replace Contents puts the new
/// art where the old art stood (#11529). `frame` is the old box, measured before the source was
/// swapped. Every instance gets its own fit, since copies of one contents differ in placement.
/// Returns what the caller must be told: a placement that had to be kept because a box couldn't be
/// measured, new contents whose shape leaves part of the frame empty, and a warp that still spans
/// the replaced contents' source pixels. A replacement that simply fills the frame says nothing.
fn fit_replaced_contents(doc: &mut Document, id: LayerId, frame: Option<[f64; 4]>) -> Result<Vec<String>> {
    let name = doc.layer(id).map(|l| l.name.clone()).unwrap_or_default();
    let kept =
        format!("layer \"{name}\": the frame of the replaced contents could not be measured, so its transform was kept: the new contents may not fit it");
    let (Some(old), Some(new)) = (frame, contents_frame(doc, smart(doc, id)?)) else { return Ok(vec![kept]) };
    let [x0, y0, x1, y1] = old;
    let [u0, v0, u1, v1] = new;
    let Some(m) = fit_map(old, new) else { return Ok(vec![kept]) };
    if already_fitted(&m) {
        return Ok(Vec::new());
    }
    let (ow, oh, nw, nh, k) = (x1 - x0, y1 - y0, u1 - u0, v1 - v0, m.m[0]);
    // The frame's empty margin after the fit: reported when the shape left a real gap, since
    // Photoshop's user would otherwise find the art smaller than the space it replaced.
    let slack = ((ow / (k * nw)) - 1.0).max((oh / (k * nh)) - 1.0);
    let warped = smart(doc, id)?.warp.is_some();
    let sm = smart_mut(doc, id)?;
    match sm.perspective {
        None => sm.transform = snap_affine(sm.transform.mul(&m)),
        Some(_) => {
            let h = placement(sm).mul(&affine_homography(&m));
            set_placement(sm, h);
        }
    }
    let mut notes = Vec::new();
    if slack > FRAME_SLACK {
        notes.push(format!(
            "layer \"{name}\": the new contents are {nw:.0} × {nh:.0} px and the frame was made for {ow:.0} × {oh:.0} px, so they are fitted inside it and don't fill it"
        ));
    }
    if warped {
        notes.push(format!("layer \"{name}\": its warp still spans the replaced contents' source pixels, so check Edit ▸ Transform ▸ Warp"));
    }
    Ok(notes)
}

pub(super) fn replace_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_param("layer.smartObjects.replaceContents", p)?.to_string();
    let bytes = photocraft_format::read_file(std::path::Path::new(&path)).map_err(|e| other(format!("can't read {path}: {e}")))?;
    let id = layer_param(s, p)?;
    replace_contents_bytes(s, id, &base_name(&path), bytes)
}

/// Replace Contents with a file the caller already read, for the smart object `p` names (`layer`,
/// else the active layer): the body of the command minus its ambient path read (#8061).
pub fn replace_contents_from_bytes(s: &mut Session, p: &Value, file_name: &str, bytes: Vec<u8>) -> Result<Value> {
    let id = layer_param(s, p)?;
    replace_contents_bytes(s, id, file_name, bytes)
}

/// Replace Contents from a file's name and bytes: what the desktop and web file pickers hand over.
pub fn replace_contents_bytes(s: &mut Session, id: LayerId, file_name: &str, bytes: Vec<u8>) -> Result<Value> {
    decode_source(file_name, &bytes)?; // fail before touching the document
    set_shared_source(s, id, "Replace Contents", false, SmartSource::Embedded { file_name: file_name.to_string(), bytes: Arc::new(bytes) }, true)
}

/// The file name and bytes of smart object `id`'s contents.
pub fn contents_of(doc: &Document, id: LayerId) -> Result<(String, Arc<Vec<u8>>)> {
    source_bytes(&doc.metadata, &smart(doc, id)?.source).ok_or_else(|| other("the smart object's contents are unavailable"))
}

/// Writes the smart object's file as it is. To a path with another extension (a `.png` for
/// `.psb` or `.pcraft` contents) the contents are opened and exported in that format instead.
pub(super) fn export_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_param("layer.smartObjects.exportContents", p)?;
    let id = layer_param(s, p)?;
    let (name, bytes) = contents_of(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    let ext = |f: &str| {
        let e = std::path::Path::new(f).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        e.map(|e| match e.as_str() {
            "jpeg" => "jpg".to_string(),
            "tiff" => "tif".to_string(),
            _ => e,
        })
    };
    if ext(path).is_none() || ext(path) == ext(&name) {
        crate::file_cmds::write_file(path, &bytes)?;
        return Ok(json!({"path": path, "fileName": name, "bytes": bytes.len(), "converted": false}));
    }
    let contents = decode_source(&name, &bytes)?;
    let r = photocraft_io::export(&contents, path, &Default::default()).map_err(|e| other(format!("can't export the contents as {}: {e}", base_name(path))))?;
    crate::file_cmds::write_file(path, &r.bytes)?;
    Ok(json!({"path": path, "fileName": name, "bytes": r.bytes.len(), "converted": true, "warnings": r.warnings}))
}

pub(super) fn edit_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let parent = st.doc.id;
    let contents_id = smart(&st.doc, id)?.contents_id;
    // Already open for editing: switch to that document rather than opening another copy.
    if let Some(index) =
        s.smart_links.iter().find(|l| l.parent == parent && l.contents_id == contents_id).and_then(|l| s.documents().iter().position(|d| d.doc.id == l.child))
    {
        s.set_active(index);
        return Ok(json!({"document": index, "parentLayer": id.0}));
    }
    let source = &smart(&st.doc, id)?.source;
    let (name, bytes) = authorized_source_bytes(s, "layer.smartObjects.editContents", &st.doc.metadata, source)?
        .ok_or_else(|| other("the smart object's contents are unavailable"))?;
    let mut child = decode_source(&name, &bytes)?;
    // Bundles keep their document id; each open copy needs its own.
    child.id = DocId::fresh();
    child.name = name;
    let index = s.add_document(child, None);
    // Admission may replace an ID already owned by another open document.
    let child_id = s.documents().get(index).ok_or(EngineError::NoDocument)?.doc.id;
    s.smart_links.retain(|l| l.child != child_id);
    s.smart_links.push(SmartLink { child: child_id, parent, layer: id, contents_id });
    Ok(json!({"document": index, "parentLayer": id.0}))
}

/// Writes an Edit Contents document back into all its parent instances (one undoable step in the
/// parent) and marks it saved. Returns false if `index` isn't an Edit Contents document.
pub fn commit_child(s: &mut Session, index: usize) -> Result<bool> {
    let Some(st) = s.docs.get(index) else { return Ok(false) };
    let child = st.doc.clone();
    let Some(link) = s.smart_links.iter().find(|l| l.child == child.id).copied() else { return Ok(false) };
    let parent = s.docs.iter().position(|d| d.doc.id == link.parent).ok_or_else(|| other("the smart object's document was closed"))?;
    let bytes = Arc::new(encode_source(&child)?);
    let prev = s.active;
    s.active = Some(parent);
    s.note_active_change();
    let r = s.edit("Edit Contents", |doc, _| {
        let ids = instances(doc, link.contents_id);
        if ids.is_empty() {
            return Err(other("the smart object's instances were removed"));
        }
        for id in ids {
            let sm = smart_mut(doc, id)?;
            let name = match &sm.source {
                SmartSource::Embedded { file_name, .. } => file_name.as_str(),
                SmartSource::Linked { .. } => child.name.as_str(),
            };
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            sm.source = SmartSource::Embedded { file_name: format!("{stem}.pcraft"), bytes: bytes.clone() };
            detach_psd(doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?);
            refresh_or_fail(doc, id)?;
        }
        Ok(())
    });
    s.active = prev;
    s.note_active_change();
    r?;
    if let Some(st) = s.docs.get_mut(index) {
        st.saved_revision = st.revision;
    }
    Ok(true)
}

/// Called before a document closes: an edited Edit Contents document updates its parent.
pub(crate) fn on_close(s: &mut Session, index: usize) {
    let Some(st) = s.docs.get(index) else { return };
    let id = st.doc.id;
    if st.is_dirty() && s.smart_links.iter().any(|l| l.child == id) {
        let _ = commit_child(s, index);
    }
    s.smart_links.retain(|l| l.child != id && l.parent != id);
    // Decoded sources are shared by content hash across documents, so the closing document's
    // can't be told apart: drop them all. Other documents decode again on their next re-render.
    if s.docs.get(index).is_some_and(|st| st.doc.walk().iter().any(|(_, _, l)| matches!(l.content, LayerContent::Smart(_)))) {
        super::purge_source_cache();
    }
}

pub(super) fn is_smart_child(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if s.smart_links.iter().any(|l| l.child == d.doc.id) { Ok(()) } else { Err("this document isn't smart object contents".into()) }
}

pub(super) fn update_all(s: &mut Session) -> Result<Value> {
    let ids: Vec<LayerId> = s
        .active()
        .ok_or(EngineError::NoDocument)?
        .doc
        .walk()
        .into_iter()
        .filter(|(_, _, l)| matches!(&l.content, LayerContent::Smart(sm) if matches!(sm.source, SmartSource::Linked { .. })))
        .map(|(_, _, l)| l.id)
        .collect();
    let mut updated = Vec::new();
    s.edit("Update All Modified Content", |doc, _| {
        for id in &ids {
            if refresh(doc, *id)? {
                updated.push(id.0);
            }
        }
        Ok(())
    })?;
    Ok(json!({"updated": updated}))
}
