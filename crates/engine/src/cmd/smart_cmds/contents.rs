use super::filters::{smart, smart_mut};
use super::placing::detach_psd;
use super::rendering::{refresh, refresh_or_fail, refresh_or_fail_from_source, unapplied_filters};
use super::source::{authorized_source_bytes, embedded_source_bytes, source_bytes};
use super::{
    Arc, DocId, Document, EngineError, LayerContent, LayerId, Metadata, Result, Session, SmartContentsId, SmartLink, SmartSource, Value, bad, base_name,
    decode_source_with_order, encode_source, json, layer_param, other,
};
use crate::jobs::{JobCtx, Started};
use photocraft_doc::HistoricalXcfSampleOrder;

pub(super) fn path_param<'a>(cmd: &str, p: &'a Value) -> Result<&'a str> {
    p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "pass `path`"))
}

/// Instances sharing contents in this document, including those inside groups.
fn instances(doc: &Document, contents_id: SmartContentsId) -> Vec<LayerId> {
    doc.walk().into_iter().filter_map(|(_, _, l)| matches!(&l.content, LayerContent::Smart(sm) if sm.contents_id == contents_id).then_some(l.id)).collect()
}

const RELINK: &str = "layer.smartObjects.relinkToFile";

/// An existing interpretation can carry over only when the picked source bytes match.
pub fn relink_sample_order(s: &Session, id: LayerId, candidate: &[u8]) -> Result<Option<HistoricalXcfSampleOrder>> {
    relink_order(s.active().ok_or(EngineError::NoDocument)?.doc(), id, candidate)
}

fn relink_order(doc: &Document, id: LayerId, candidate: &[u8]) -> Result<Option<HistoricalXcfSampleOrder>> {
    let old = smart(doc, id)?;
    let Some(order) = old.historical_xcf_sample_order else { return Ok(None) };
    let same = if let Some(digest) = old.historical_xcf_link_digest {
        digest == photocraft_io::linked::historical_xcf_digest(candidate)
    } else {
        embedded_source_bytes(&doc.metadata, &old.source).is_some_and(|(_, bytes)| bytes.as_slice() == candidate)
    };
    Ok(same.then_some(order))
}

pub(super) fn relink_to_file(s: &mut Session, p: &Value) -> Result<Value> {
    let explicit = p.get("historicalXcfSampleOrder").map(|_| crate::file_cmds::historical_xcf_sample_order(p, RELINK)).transpose()?;
    let expected = p
        .get("expectedSourceDigest")
        .filter(|value| !value.is_null())
        .map(|value| <[u8; 32] as serde::Deserialize>::deserialize(value).map_err(|_| bad(RELINK, "expectedSourceDigest must be an array of 32 bytes")))
        .transpose()?;
    let path = path_param(RELINK, p)?.to_string();
    let id = layer_param(s, p)?;
    let snapshot = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    smart(&snapshot, id)?;
    let label = "Relink to File";
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            let change = read_relink_source(&snapshot, id, path, explicit, expected, ctx)?;
            prepare_source_change(&snapshot, id, change, ctx)
        },
        move |s, updated| publish_source_change(s, id, label, updated),
    )
}

fn read_relink_source(
    doc: &Document,
    id: LayerId,
    path: String,
    explicit: Option<Option<HistoricalXcfSampleOrder>>,
    expected: Option<[u8; 32]>,
    ctx: &JobCtx,
) -> Result<SourceChange> {
    ctx.check()?;
    crate::jobs::authorize_render_command("layer.smartObjects.updateModifiedContent", &json!({"path":path}))?;
    let resolved = crate::doc_path::resolve(&path).map_err(|error| crate::doc_path::refusal("link", &path, error))?;
    let bytes = photocraft_format::read_file(&resolved).map_err(|error| other(format!("can't read {path}: {error}")))?;
    ctx.check()?;
    let verified = expected.map(|_| photocraft_io::linked::historical_xcf_digest(&bytes));
    if expected != verified {
        return Err(other("the linked source changed after it was picked; select it again"));
    }
    let order = match explicit {
        Some(explicit) => explicit,
        None => relink_order(doc, id, &bytes)?,
    };
    let order = picked_order(&path, &bytes, order, ctx)?;
    let digest = order.map(|_| verified.unwrap_or_else(|| photocraft_io::linked::historical_xcf_digest(&bytes)));
    let snapshot = Some((base_name(&path), Arc::new(bytes)));
    Ok(SourceChange { source: SmartSource::Linked { path }, order, digest, snapshot })
}

/// Swaps shared contents, keeping each instance's transform and filters, and re-renders.
/// Storage conversions preserve the interpretation of unchanged source bytes.
pub(super) fn set_source(
    s: &mut Session,
    p: &Value,
    label: &str,
    keep_psd: bool,
    make: impl FnOnce(&Metadata, &photocraft_doc::SmartObject) -> Result<SmartSource>,
) -> Result<Value> {
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let old = smart(doc, id)?;
    let order = old.historical_xcf_sample_order;
    let new = make(&doc.metadata, old)?;
    let digest = if order.is_some() && matches!(new, SmartSource::Linked { .. }) {
        old.historical_xcf_link_digest.or_else(|| embedded_source_bytes(&doc.metadata, &old.source).map(|(_, bytes)| *blake3::hash(&bytes).as_bytes()))
    } else {
        None
    };
    set_shared_source(s, id, label, keep_psd, new, order, digest)
}

// Keep the shared-instance transaction out of the per-command source-builder instantiations.
fn set_shared_source(
    s: &mut Session,
    id: LayerId,
    label: &str,
    keep_psd: bool,
    new: SmartSource,
    historical_xcf_sample_order: Option<photocraft_doc::HistoricalXcfSampleOrder>,
    historical_xcf_link_digest: Option<[u8; 32]>,
) -> Result<Value> {
    let change = SourceChange { source: new, order: historical_xcf_sample_order, digest: historical_xcf_link_digest, snapshot: None };
    let change = s.edit(label, |doc, _| apply_source_change(doc, id, keep_psd, &change, &JobCtx::new()))?;
    Ok(report_source_change(s, id, change))
}

struct SourceChange {
    source: SmartSource,
    order: Option<HistoricalXcfSampleOrder>,
    digest: Option<[u8; 32]>,
    snapshot: Option<(String, Arc<Vec<u8>>)>,
}

struct ChangedInstances {
    parent: DocId,
    contents_id: SmartContentsId,
    layers: Vec<LayerId>,
    warnings: std::collections::BTreeSet<String>,
}

fn apply_source_change(doc: &mut Document, id: LayerId, keep_psd: bool, change: &SourceChange, ctx: &JobCtx) -> Result<ChangedInstances> {
    let contents_id = smart(doc, id)?.contents_id;
    let layers = instances(doc, contents_id);
    let mut warnings = std::collections::BTreeSet::new();
    for &layer in &layers {
        ctx.check()?;
        let sm = smart_mut(doc, layer)?;
        sm.source = change.source.clone();
        sm.historical_xcf_sample_order = change.order;
        sm.historical_xcf_link_digest = change.digest;
        if !keep_psd {
            detach_psd(doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?);
        }
        warnings.extend(unapplied_filters(smart(doc, layer)?).into_iter().map(|n| format!("the {n} smart filter is not supported, so it was not applied")));
        refresh_or_fail_from_source(doc, layer, ctx, change.snapshot.clone())?;
    }
    Ok(ChangedInstances { parent: doc.id, contents_id, layers, warnings })
}

fn report_source_change(s: &mut Session, id: LayerId, change: ChangedInstances) -> Value {
    let ChangedInstances { parent, contents_id, layers, warnings } = change;
    // An old editor must not overwrite newly replaced/relinked contents on close. Keep its
    // document open, but the next Edit Contents opens the replacement source.
    s.smart_links.retain(|l| l.parent != parent || l.contents_id != contents_id);
    // Every instance changed, not only `layer`: list them, so a caller doesn't repeat the
    // command once per copy (#4075).
    json!({"layer": id.0, "instances": layers.iter().map(|l| l.0).collect::<Vec<_>>(), "warnings": warnings})
}

/// Finish a picker-confirmed source change on a native worker. Publish the refreshed
/// document and invalidate old Edit Contents children only after successful rendering.
pub fn start_picked_replacement(s: &mut Session, id: LayerId, name: String, bytes: Vec<u8>, order: Option<HistoricalXcfSampleOrder>) -> Result<Started> {
    let command = "layer.smartObjects.replaceContents";
    if let Some(conflict) = s.job_conflict(command, true) {
        return Err(other(conflict));
    }
    let label = "Replace Contents";
    let snapshot = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    smart(&snapshot, id)?;
    s.start_job(
        command,
        json!({"layer":id.0,"historicalXcfSampleOrder":order}),
        label,
        true,
        move |ctx| {
            let order = picked_order(&name, &bytes, order, ctx)?;
            let source = SmartSource::Embedded { file_name: name, bytes: Arc::new(bytes) };
            prepare_source_change(&snapshot, id, SourceChange { source, order, digest: None, snapshot: None }, ctx)
        },
        move |s, updated| publish_source_change(s, id, label, updated),
    )
}

fn picked_order(name: &str, bytes: &[u8], order: Option<HistoricalXcfSampleOrder>, ctx: &JobCtx) -> Result<Option<HistoricalXcfSampleOrder>> {
    ctx.stage(0., 0.05, "Reading source", |ctl| photocraft_io::applicable_historical_xcf_sample_order(name, bytes, order, ctl))
        .map_err(|error| crate::error::io_open_error(name, error))
}

fn prepare_source_change(snapshot: &Document, id: LayerId, change: SourceChange, ctx: &JobCtx) -> Result<(Document, ChangedInstances)> {
    let mut doc = snapshot.clone();
    let changed = apply_source_change(&mut doc, id, false, &change, ctx)?;
    ctx.check()?;
    Ok((doc, changed))
}

fn publish_source_change(s: &mut Session, id: LayerId, label: &str, (updated, changed): (Document, ChangedInstances)) -> Result<Value> {
    s.edit(label, move |doc, _| {
        *doc = updated;
        Ok(())
    })?;
    Ok(report_source_change(s, id, changed))
}

pub(super) fn replace_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let order = crate::file_cmds::historical_xcf_sample_order(p, "layer.smartObjects.replaceContents")?;
    let path = path_param("layer.smartObjects.replaceContents", p)?.to_string();
    let bytes = photocraft_format::read_file(std::path::Path::new(&path)).map_err(|e| other(format!("can't read {path}: {e}")))?;
    let id = layer_param(s, p)?;
    replace_contents_bytes_with_order(s, id, &base_name(&path), bytes, order)
}

/// Replace Contents with a file the caller already read, for the smart object `p` names (`layer`,
/// else the active layer): the body of the command minus its ambient path read (#8061).
pub fn replace_contents_from_bytes(s: &mut Session, p: &Value, file_name: &str, bytes: Vec<u8>) -> Result<Value> {
    let order = crate::file_cmds::historical_xcf_sample_order(p, "layer.smartObjects.replaceContents")?;
    let id = layer_param(s, p)?;
    replace_contents_bytes_with_order(s, id, file_name, bytes, order)
}

/// Replace Contents from a file's name and bytes: what the desktop and web file pickers hand over.
pub fn replace_contents_bytes(s: &mut Session, id: LayerId, file_name: &str, bytes: Vec<u8>) -> Result<Value> {
    replace_contents_bytes_with_order(s, id, file_name, bytes, None)
}

fn replace_contents_bytes_with_order(
    s: &mut Session,
    id: LayerId,
    file_name: &str,
    bytes: Vec<u8>,
    order: Option<photocraft_doc::HistoricalXcfSampleOrder>,
) -> Result<Value> {
    decode_source_with_order(file_name, &bytes, order)?; // fail before touching the document
    let order = photocraft_io::applicable_historical_xcf_sample_order(file_name, &bytes, order, &photocraft_raster::Interrupt::NONE)
        .map_err(|error| other(error.to_string()))?;
    set_shared_source(s, id, "Replace Contents", false, SmartSource::Embedded { file_name: file_name.to_string(), bytes: Arc::new(bytes) }, order, None)
}

/// The file name and bytes of smart object `id`'s contents.
pub fn contents_of(doc: &Document, id: LayerId) -> Result<(String, Arc<Vec<u8>>)> {
    let object = smart(doc, id)?;
    let (name, bytes) = source_bytes(&doc.metadata, &object.source).ok_or_else(|| other("the smart object's contents are unavailable"))?;
    photocraft_io::linked::check_historical_xcf_digest(object.historical_xcf_link_digest, &bytes).map_err(EngineError::HistoricalXcfLinkChanged)?;
    Ok((name, bytes))
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
    let source = smart(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    let contents = decode_source_with_order(&name, &bytes, source.historical_xcf_sample_order)?;
    let r = photocraft_io::export(&contents, path, &photocraft_io::ExportOptions { read_linked_files: s.authorize.is_none(), ..Default::default() })
        .map_err(|e| other(format!("can't export the contents as {}: {e}", base_name(path))))?;
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
    let sm = smart(&st.doc, id)?;
    let source = &sm.source;
    let (name, bytes) = authorized_source_bytes(s, "layer.smartObjects.editContents", &st.doc.metadata, source)?
        .ok_or_else(|| other("the smart object's contents are unavailable"))?;
    photocraft_io::linked::check_historical_xcf_digest(sm.historical_xcf_link_digest, &bytes).map_err(other)?;
    let mut child = decode_source_with_order(&name, &bytes, sm.historical_xcf_sample_order)?;
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
            sm.historical_xcf_sample_order = None;
            sm.historical_xcf_link_digest = None;
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

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "contents/historical_order_tests.rs"]
mod historical_order_tests;
