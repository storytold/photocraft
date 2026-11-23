//! Private worker preparation. No source or cache enters the session before success.
use super::{
    Arc, Document, EngineError, LayerId, Result, Session, SmartContentsId, SmartSource, Value, base_name, decode_source, detach_psd, instances, json, other,
    smart, smart_mut, unapplied_filters,
};
use crate::jobs::{JobCtx, Started};

const COMMAND: &str = "layer.smartObjects.replaceContents";
const LABEL: &str = "Replace Contents";

pub(super) enum Source {
    Path(String),
    Bytes(String, Arc<Vec<u8>>),
}

struct Prepared {
    document: Document,
    layer: LayerId,
    contents: SmartContentsId,
    changed: Vec<LayerId>,
    warnings: std::collections::BTreeSet<String>,
}

fn capture(s: &Session, id: LayerId) -> Result<Arc<Document>> {
    let document = s.active().ok_or(EngineError::NoDocument)?.doc().clone();
    smart(&document, id)?;
    Ok(document)
}

fn prepare(snapshot: Arc<Document>, id: LayerId, source: Source, ctx: &JobCtx) -> Result<Prepared> {
    ctx.check()?;
    ctx.progress(0., "Reading contents");
    let (name, bytes) = match source {
        Source::Path(path) => {
            let bytes = photocraft_format::read_file(std::path::Path::new(&path)).map_err(|e| other(format!("can't read {path}: {e}")))?;
            (base_name(&path), Arc::new(bytes))
        }
        Source::Bytes(name, bytes) => (name, bytes),
    };
    ctx.check()?;
    ctx.progress(0.1, "Decoding contents");
    // Strict SVG/SVGZ validation and ordinary engine import admission stay central.
    // An omitted image or refused import cannot replace any instance's source.
    decode_source(&name, &bytes)?;
    ctx.check()?;
    let mut document = (*snapshot).clone();
    let contents = smart(&document, id)?.contents_id;
    let changed = instances(&document, contents);
    let source = SmartSource::Embedded { file_name: name, bytes };
    let mut warnings = std::collections::BTreeSet::new();
    for (index, &instance) in changed.iter().enumerate() {
        ctx.check()?;
        ctx.progress(0.2 + 0.8 * index as f32 / changed.len().max(1) as f32, "Preparing contents");
        let object = smart_mut(&mut document, instance)?;
        object.source = source.clone();
        // A fresh replacement owns its interpretation. Never carry another
        // source's byte order or linked-byte approval into its cold renders.
        object.historical_xcf_sample_order = None;
        object.historical_xcf_link_digest = None;
        detach_psd(document.layer_mut(instance).ok_or(EngineError::NoLayer(instance))?);
        warnings.extend(
            unapplied_filters(smart(&document, instance)?).into_iter().map(|n| format!("the {n} smart filter is not supported, so it was not applied")),
        );
        super::super::rendering::refresh_or_fail_in(&mut document, instance, ctx)?;
    }
    ctx.check()?;
    Ok(Prepared { document, layer: id, contents, changed, warnings })
}

fn apply(s: &mut Session, prepared: Prepared) -> Result<Value> {
    let parent = prepared.document.id;
    let contents = prepared.contents;
    if s.active().is_none_or(|state| state.doc.id != parent) {
        return Err(other("the replacement document is no longer active"));
    }
    s.edit(LABEL, |document, _| {
        *document = prepared.document;
        Ok(())
    })?;
    // Existing editors may stay open, but must not overwrite replaced contents.
    s.smart_links.retain(|link| link.parent != parent || link.contents_id != contents);
    Ok(json!({"layer": prepared.layer.0, "instances": prepared.changed.iter().map(|id| id.0).collect::<Vec<_>>(), "warnings": prepared.warnings}))
}

pub(super) fn run(s: &mut Session, id: LayerId, source: Source) -> Result<Value> {
    let snapshot = capture(s, id)?;
    crate::jobs::run(s, LABEL, true, move |ctx| prepare(snapshot, id, source, ctx), apply)
}

pub(super) fn start(s: &mut Session, id: LayerId, name: &str, bytes: Vec<u8>) -> Result<Started> {
    let params = json!({"layer":id.0,"path":name});
    s.validate_params(COMMAND, crate::registry::spec(COMMAND)?, &params)?;
    if let Some(authorize) = s.authorize {
        authorize(COMMAND, &params)?;
    }
    if let Some(conflict) = s.job_conflict(COMMAND, true) {
        return Err(other(conflict));
    }
    let snapshot = capture(s, id)?;
    let source = Source::Bytes(name.into(), Arc::new(bytes));
    s.start_job(COMMAND, params, LABEL, true, move |ctx| prepare(snapshot, id, source, ctx), apply)
}

#[cfg(test)]
mod tests;
