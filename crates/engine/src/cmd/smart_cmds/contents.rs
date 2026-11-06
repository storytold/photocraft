use super::filters::{smart, smart_mut};
use super::placing::detach_psd;
use super::rendering::{refresh, refresh_or_fail, unapplied_filters};
use super::source::{authorized_source_bytes, source_bytes};
use super::{
    Arc, DocId, Document, EngineError, LayerContent, LayerId, Metadata, Result, Session, SmartContentsId, SmartLink, SmartSource, Value, bad, base_name,
    decode_source, encode_source, json, layer_param, other,
};

pub(super) fn path_param<'a>(cmd: &str, p: &'a Value) -> Result<&'a str> {
    p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "pass `path`"))
}

/// Instances sharing contents in this document, including those inside groups.
fn instances(doc: &Document, contents_id: SmartContentsId) -> Vec<LayerId> {
    doc.walk().into_iter().filter_map(|(_, _, l)| matches!(&l.content, LayerContent::Smart(sm) if sm.contents_id == contents_id).then_some(l.id)).collect()
}

/// Swaps shared contents, keeping each instance's transform and filters, and re-renders.
pub(super) fn set_source(
    s: &mut Session,
    p: &Value,
    label: &str,
    keep_psd: bool,
    validate_contents: bool,
    make: impl FnOnce(&Metadata, &SmartSource) -> Result<SmartSource>,
) -> Result<Value> {
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let pdf_import = smart(doc, id)?.pdf_import;
    let new = make(&doc.metadata, &smart(doc, id)?.source)?;
    set_shared_source(s, id, label, keep_psd, new, pdf_import, validate_contents)
}

/// Validates readable replacement contents and reports whether their source is a PDF.
/// Unavailable linked sources are classified by extension without attempting a decode.
fn validate_replacement(meta: &Metadata, source: &SmartSource, pdf_import: Option<super::PdfImportOptions>) -> Result<bool> {
    let Some((file_name, bytes)) = source_bytes(meta, source) else {
        return Ok(matches!(source, SmartSource::Linked { path } if std::path::Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))));
    };
    let is_pdf = photocraft_io::is_pdf_input(&file_name, &bytes);
    decode_source(&file_name, &bytes, pdf_import.filter(|_| is_pdf))?;
    Ok(is_pdf)
}

/// Replaces every shared instance in one undoable edit, preserving placement and filters.
/// Optionally validates the new source, clears incompatible PDF options, and detaches old editors.
fn set_shared_source(
    s: &mut Session,
    id: LayerId,
    label: &str,
    keep_psd: bool,
    new: SmartSource,
    target_pdf_import: Option<super::PdfImportOptions>,
    validate_contents: bool,
) -> Result<Value> {
    // Embedding/linking the same source preserves its projection and preview without decoding
    // an otherwise unsupported native vector format (EPS/AI).
    let clear_pdf_import = validate_contents && !validate_replacement(&s.active().ok_or(EngineError::NoDocument)?.doc.metadata, &new, target_pdf_import)?;
    let (parent, contents_id, changed, warnings) = s.edit(label, |doc, _| {
        let contents_id = smart(doc, id)?.contents_id;
        let changed = instances(doc, contents_id);
        let mut warnings = std::collections::BTreeSet::new();
        for &instance in &changed {
            let sm = smart_mut(doc, instance)?;
            sm.source = new.clone();
            if clear_pdf_import {
                sm.pdf_import = None;
            }
            if !keep_psd {
                detach_psd(doc.layer_mut(instance).ok_or(EngineError::NoLayer(instance))?);
            }
            warnings
                .extend(unapplied_filters(smart(doc, instance)?).into_iter().map(|n| format!("the {n} smart filter is not supported, so it was not applied")));
            refresh_or_fail(doc, instance)?;
        }
        Ok((doc.id, contents_id, changed, warnings))
    })?;
    // An old editor must not overwrite newly replaced/relinked contents on close. Keep its
    // document open, but the next Edit Contents opens the replacement source.
    s.smart_links.retain(|link| link.parent != parent || (link.contents_id != contents_id && !changed.contains(&link.layer)));
    // Every instance changed, not only `layer`: list them, so a caller doesn't repeat the
    // command once per copy (#4075).
    Ok(json!({"layer": id.0, "instances": changed.iter().map(|l| l.0).collect::<Vec<_>>(), "warnings": warnings}))
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
    let pdf_import = smart(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?.pdf_import;
    set_shared_source(s, id, "Replace Contents", false, SmartSource::Embedded { file_name: file_name.to_string(), bytes: Arc::new(bytes) }, pdf_import, true)
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
    let pdf_import = smart(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?.pdf_import;
    let contents = decode_source(&name, &bytes, pdf_import)?;
    let r = photocraft_io::export(&contents, path, &Default::default()).map_err(|e| other(format!("can't export the contents as {}: {e}", base_name(path))))?;
    crate::file_cmds::write_file(path, &r.bytes)?;
    Ok(json!({"path": path, "fileName": name, "bytes": r.bytes.len(), "converted": true, "warnings": r.warnings}))
}

/// Activates an existing editor for this source projection or opens authorized contents in a new one.
/// Distinct PDF projections receive separate editors even when their source identity is shared.
pub(super) fn edit_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let parent = st.doc.id;
    let sm = smart(&st.doc, id)?;
    let contents_id = sm.contents_id;
    let pdf_import = sm.pdf_import.unwrap_or_default();
    // A document edits one source projection. Different selected PDF pages stay independent.
    if let Some(index) = s.smart_links.iter().find(|link| link.matches(parent, sm)).and_then(|link| s.documents().iter().position(|d| d.doc.id == link.child)) {
        s.set_active(index);
        return Ok(json!({"document": index, "parentLayer": id.0}));
    }
    let source = &sm.source;
    let (name, bytes) = authorized_source_bytes(s, "layer.smartObjects.editContents", &st.doc.metadata, source)?
        .ok_or_else(|| other("the smart object's contents are unavailable"))?;
    let mut child = decode_source(&name, &bytes, sm.pdf_import)?;
    // Bundles keep their document id; each open copy needs its own.
    child.id = DocId::fresh();
    child.name = name;
    let index = s.add_document(child, None);
    // Admission may replace an ID already owned by another open document.
    let child_id = s.documents().get(index).ok_or(EngineError::NoDocument)?.doc.id;
    s.smart_links.retain(|l| l.child != child_id);
    s.smart_links.push(SmartLink { child: child_id, parent, layer: id, contents_id, original_contents_id: contents_id, pdf_import });
    Ok(json!({"document": index, "parentLayer": id.0}))
}

/// Writes an Edit Contents document back into matching parent projections (one undoable step in
/// the parent) and marks it saved. Returns false if `index` isn't an Edit Contents document.
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
        // Parent undo may restore the original PDF identity after a projection was detached.
        let contents_id = match doc.layer(link.layer).map(|layer| &layer.content) {
            Some(LayerContent::Smart(sm)) if sm.contents_id == link.original_contents_id && sm.pdf_import.unwrap_or_default() == link.pdf_import => {
                link.original_contents_id
            }
            _ => link.contents_id,
        };
        let mut has_other_projection = false;
        let ids: Vec<LayerId> = doc
            .walk()
            .into_iter()
            .filter_map(|(_, _, layer)| match &layer.content {
                LayerContent::Smart(sm) if sm.contents_id == contents_id && link.matches(doc.id, sm) => Some(layer.id),
                LayerContent::Smart(sm) if sm.contents_id == contents_id => {
                    has_other_projection = true;
                    None
                }
                _ => None,
            })
            .collect();
        if ids.is_empty() {
            return Err(other("the smart object's instances were removed"));
        }
        // Native contents have no PDF options. Give a nondefault projection its own identity
        // even without siblings, so repeat saves and parent undo/redo can distinguish it from
        // an unprojected PDF in the original family.
        let materializes_projection = contents_id == link.original_contents_id && link.pdf_import != Default::default();
        let detached_contents_id = (has_other_projection || materializes_projection).then(SmartContentsId::fresh);
        for id in ids {
            let sm = smart_mut(doc, id)?;
            let name = match &sm.source {
                SmartSource::Embedded { file_name, .. } => file_name.as_str(),
                SmartSource::Linked { .. } => child.name.as_str(),
            };
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            sm.source = SmartSource::Embedded { file_name: format!("{stem}.pcraft"), bytes: bytes.clone() };
            sm.pdf_import = None;
            if let Some(contents_id) = detached_contents_id {
                sm.contents_id = contents_id;
            }
            detach_psd(doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?);
            refresh_or_fail(doc, id)?;
        }
        Ok(detached_contents_id.unwrap_or(contents_id))
    });
    s.active = prev;
    s.note_active_change();
    let contents_id = r?;
    if let Some(st) = s.docs.get_mut(index) {
        st.saved_revision = st.revision;
    }
    if let Some(link) = s.smart_links.iter_mut().find(|candidate| candidate.child == child.id) {
        link.contents_id = contents_id;
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
