//! Linked choices bind to a verified encoded revision. Transactions render from one read.
use super::{
    Arc, Document, EngineError, LayerId, Result, Session, SmartContentsId, SmartSource, Value, base_name, contents_of, detach_psd, instances, json, layer_param,
    other, path_param, smart, smart_mut, unapplied_filters,
};
use crate::jobs::JobCtx;
use photocraft_doc::HistoricalXcfSampleOrder as Order;

const RELINK: &str = "layer.smartObjects.relinkToFile";

fn order_from_io(order: photocraft_io::HistoricalXcfSampleOrder) -> Order {
    match order {
        photocraft_io::HistoricalXcfSampleOrder::BigEndian => Order::BigEndian,
        photocraft_io::HistoricalXcfSampleOrder::LittleEndian => Order::LittleEndian,
    }
}
fn order_to_io(order: Order) -> photocraft_io::HistoricalXcfSampleOrder {
    match order {
        Order::BigEndian => photocraft_io::HistoricalXcfSampleOrder::BigEndian,
        Order::LittleEndian => photocraft_io::HistoricalXcfSampleOrder::LittleEndian,
    }
}

/// A saved interpretation carries over only when the candidate is the exact old revision.
fn inherited_order(doc: &Document, id: LayerId, bytes: &[u8]) -> Result<Option<Order>> {
    let old = smart(doc, id)?;
    let Some(order) = old.historical_xcf_sample_order else { return Ok(None) };
    let same = match old.historical_xcf_link_digest {
        Some(digest) => digest == photocraft_io::linked::historical_xcf_digest(bytes),
        None => super::super::source::embedded_source_bytes(&doc.metadata, &old.source).is_some_and(|(_, old)| old.as_slice() == bytes),
    };
    Ok(same.then_some(order))
}

struct Change {
    path: String,
    snapshot: (String, Arc<Vec<u8>>),
    order: Option<Order>,
    digest: Option<[u8; 32]>,
}
fn read_change(doc: &Document, id: LayerId, path: String, explicit: Option<Option<Order>>, expected: Option<[u8; 32]>, ctx: &JobCtx) -> Result<Change> {
    ctx.check()?;
    crate::jobs::authorize_render_command("layer.smartObjects.updateModifiedContent", &json!({"path":path}))?;
    let resolved = crate::doc_path::resolve(&path).map_err(|e| crate::doc_path::refusal("link", &path, e))?;
    let bytes = photocraft_format::read_file(&resolved).map_err(|e| other(format!("can't read {path}: {e}")))?;
    ctx.check()?;
    let hash = photocraft_io::linked::historical_xcf_digest(&bytes);
    if expected.is_some_and(|expected| expected != hash) {
        return Err(EngineError::SourceChanged { name: path });
    }
    let requested = match explicit {
        Some(order) => order,
        None => inherited_order(doc, id, &bytes)?,
    };
    let policy = requested.map_or(photocraft_io::HistoricalXcfOrder::RequireExplicit, |order| photocraft_io::HistoricalXcfOrder::Explicit(order_to_io(order)));
    let name = base_name(&path);
    crate::memory_gate::ensure(&format!("render the smart object “{name}”"), crate::memory_gate::smart_render_need(&bytes))?;
    let imported = ctx.stage(0.05, 0.5, "Reading source", |ctl| {
        crate::file_cmds::import_result_with_interrupt(&name, &bytes, &photocraft_io::ImportOptions { historical_xcf_order: policy, ..Default::default() }, ctl)
    })?;
    ctx.check()?;
    let order = imported.historical_xcf_sample_order.map(order_from_io);
    // This is the same admission/decode path used by cold Smart rendering. Reuse its pixels,
    // while the strict SVG importer and Stack Mode keep their own independent paths.
    super::super::source::prime_imported_source(&name, &bytes, doc.pixel_format(), order, &imported.document)?;
    ctx.check()?;
    Ok(Change { path, snapshot: (name, Arc::new(bytes)), order, digest: order.map(|_| hash) })
}
struct PreparedChange {
    document: Document,
    contents: SmartContentsId,
    layers: Vec<LayerId>,
    warnings: std::collections::BTreeSet<String>,
}
fn prepare_change(doc: &Document, id: LayerId, change: &Change, ctx: &JobCtx) -> Result<PreparedChange> {
    ctx.check()?;
    let contents = smart(doc, id)?.contents_id;
    let layers = instances(doc, contents);
    let mut document = doc.clone();
    let mut warnings = std::collections::BTreeSet::new();
    for &id in &layers {
        ctx.check()?;
        warnings.extend(
            unapplied_filters(smart(&document, id)?).into_iter().map(|name| format!("the {name} smart filter is not supported, so it was not applied")),
        );
        let object = smart_mut(&mut document, id)?;
        object.source = SmartSource::Linked { path: change.path.clone() };
        object.historical_xcf_sample_order = change.order;
        object.historical_xcf_link_digest = change.digest;
        detach_psd(document.layer_mut(id).ok_or(EngineError::NoLayer(id))?);
        super::super::rendering::refresh_or_fail_from_source(&mut document, id, ctx, change.snapshot.clone())?;
    }
    ctx.check()?;
    Ok(PreparedChange { document, contents, layers, warnings })
}
fn publish(s: &mut Session, id: LayerId, label: &str, prepared: PreparedChange) -> Result<Value> {
    let PreparedChange { document, contents, layers, warnings } = prepared;
    let parent = document.id;
    s.edit(label, move |current, _| {
        if current.id != parent {
            return Err(other("the source document is no longer active"));
        }
        *current = document;
        Ok(())
    })?;
    s.smart_links.retain(|link| link.parent != parent || link.contents_id != contents);
    s.link_scan = None;
    Ok(json!({"layer":id.0,"instances":layers.iter().map(|id|id.0).collect::<Vec<_>>(),"warnings":warnings}))
}

pub(crate) fn relink_to_file(s: &mut Session, p: &Value) -> Result<Value> {
    // All schema checks precede filesystem work; null explicitly declines old inheritance.
    let parsed = crate::file_cmds::historical_xcf_sample_order(p, RELINK)?.map(order_from_io);
    let explicit = p.get("historicalXcfSampleOrder").map(|_| parsed);
    let expected = crate::file_cmds::expected_source_digest(p, RELINK)?;
    let path = path_param(RELINK, p)?.to_string();
    let path = if std::path::Path::new(&path).is_relative() {
        std::path::absolute(&path).map_err(|e| other(format!("can't resolve {path}: {e}")))?.to_string_lossy().into_owned()
    } else {
        path
    };
    let id = layer_param(s, p)?;
    let source = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    smart(&source, id)?;
    crate::jobs::run(
        s,
        "Relink to File",
        true,
        move |ctx| {
            let change = read_change(&source, id, path, explicit, expected, ctx)?;
            prepare_change(&source, id, &change, ctx)
        },
        move |s, prepared| publish(s, id, "Relink to File", prepared),
    )
}

pub(crate) fn convert_to_linked(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_param("layer.smartObjects.convertToLinked", p)?.to_string();
    let path = if std::path::Path::new(&path).is_relative() {
        std::path::absolute(&path).map_err(|e| other(format!("can't resolve {path}: {e}")))?.to_string_lossy().into_owned()
    } else {
        path
    };
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let old = smart(doc, id)?;
    let order = old.historical_xcf_sample_order;
    let (_, bytes) = contents_of(doc, id)?; // rechecks any old linked digest before the first write
    let snapshot = (base_name(&path), bytes);
    let digest = order.map(|_| photocraft_io::linked::historical_xcf_digest(&snapshot.1));
    let change = Change { path: path.clone(), snapshot, order, digest };
    let prepared = prepare_change(doc, id, &change, &JobCtx::new())?;
    crate::file_cmds::write_file(&path, &change.snapshot.1)?;
    publish(s, id, "Convert to Linked", prepared)
}

#[cfg(test)]
mod tests;
