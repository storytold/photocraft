//! Fresh sample-order choices for exact picked replacement bytes, never for an old source.
use crate::{PhotocraftApp, open_prompt::PromptKind};
use photocraft_doc::{DocId, LayerContent, LayerId, SmartContentsId};
use photocraft_engine::jobs::{JobCtx, JobEvent, JobId, JobOutcome, Started, StepWork};
use serde_json::{Value, json};
use std::sync::Arc;

const FIELD: &str = "__historicalXcfReplacement";
const COMMAND: &str = "layer.smartObjects.replaceContents";

#[derive(Clone, Copy)]
pub(crate) struct Target {
    doc: DocId,
    layer: LayerId,
    contents: SmartContentsId,
    revision: u64,
}

struct Held {
    id: u64,
    target: Target,
    name: String,
    bytes: Arc<Vec<u8>>,
    question: Option<photocraft_io::HistoricalXcfSampleOrderRequest>,
    job: Option<JobId>,
}

#[derive(Default)]
pub(crate) struct Pending {
    next: u64,
    held: Vec<Held>,
}

impl Pending {
    pub(crate) fn len(&self) -> usize {
        self.held.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

pub(crate) fn target(app: &PhotocraftApp) -> Result<Target, String> {
    let state = app.session.active().ok_or("no document")?;
    let layer = state.active_layer.ok_or("no active layer")?;
    let Some(LayerContent::Smart(smart)) = state.doc.layer(layer).map(|layer| &layer.content) else { return Err("the layer is not a smart object".into()) };
    Ok(Target { doc: state.doc.id, layer, contents: smart.contents_id, revision: state.revision })
}

fn check(app: &PhotocraftApp, target: Target) -> Result<(), String> {
    let state = app.session.documents().iter().find(|state| state.doc.id == target.doc).ok_or("the replacement document was closed")?;
    let Some(LayerContent::Smart(smart)) = state.doc.layer(target.layer).map(|layer| &layer.content) else {
        return Err("the replacement target is no longer a smart object".into());
    };
    // FIFO processing does not renew permission to overwrite a changed target. An
    // earlier accepted replacement makes later picks stale; cancelling it does not.
    if state.revision != target.revision || smart.contents_id != target.contents {
        return Err("the replacement target changed; pick its contents again".into());
    }
    Ok(())
}

struct ProbeWork {
    name: String,
    bytes: Arc<Vec<u8>>,
}

impl StepWork<Option<photocraft_io::HistoricalXcfSampleOrderRequest>> for ProbeWork {
    fn step(
        &mut self,
        ctx: &JobCtx,
        _deadline: web_time::Instant,
    ) -> photocraft_engine::Result<Option<Option<photocraft_io::HistoricalXcfSampleOrderRequest>>> {
        ctx.check()?;
        // Codec header parsing is one opaque unit, deferred on cooperative hosts and
        // performed on a worker on desktop. Cancellation reaches every input read.
        let question = ctx
            .stage(0., 1., "Reading replacement header", |interrupt| {
                photocraft_io::historical_xcf_sample_order_request_with(&self.name, &self.bytes, interrupt)
            })
            .map_err(|error| match error {
                photocraft_io::IoError::Cancelled => photocraft_engine::EngineError::Cancelled,
                error => photocraft_engine::EngineError::Other(error.to_string()),
            })?;
        ctx.check()?;
        Ok(Some(question))
    }
}

pub(crate) fn pick(app: &mut PhotocraftApp, target: Target, name: String, bytes: Vec<u8>) -> Result<Value, String> {
    check(app, target)?;
    let wrapped = photocraft_engine::jobs::has_compressed_import_signature(&bytes);
    // Even a fixed-output probe can scan an arbitrarily long compressed header.
    // Only plain fixed headers are inspected in the picker callback.
    let question = if wrapped { None } else { photocraft_io::historical_xcf_sample_order_request(&name, &bytes).map_err(|error| error.to_string())? };
    // Plain modern sources reserve their turn just like probed wrappers. Applying
    // one now would change the revision underneath an earlier unanswered choice.
    if !wrapped && question.is_none() && app.prompt_order.is_empty() && !crate::open_prompt::waiting(app) {
        return replace(app, target, &name, bytes, None);
    }
    let pending = &mut app.historical_replacements;
    let retained = pending.held.iter().try_fold(bytes.capacity(), |total, held| total.checked_add(held.bytes.capacity()));
    if pending.held.len() >= 8 || retained.is_none_or(|bytes| bytes > 512 << 20) {
        return Err("too many replacement sources await a sample-order choice; finish or cancel an earlier choice".into());
    }
    let id = pending.next.checked_add(1).ok_or("replacement source identity limit reached")?;
    pending.next = id;
    let bytes = Arc::new(bytes);
    pending.held.push(Held { id, target, name: name.clone(), bytes: bytes.clone(), question, job: None });
    app.prompt_order.push_back(PromptKind::Replacement(id));
    if wrapped {
        let started = app.session.start_job_stepped(
            COMMAND,
            json!({"layer":target.layer.0}),
            "Reading replacement header",
            false,
            ProbeWork { name, bytes },
            |_, question| Ok(json!({"historical":question.map(|question| json!({"version":question.version,"bitsPerSample":question.bits_per_sample}))})),
        );
        match started {
            Ok(Started::Job(job)) => {
                if let Some(held) = app.historical_replacements.held.iter_mut().find(|held| held.id == id) {
                    held.job = Some(job);
                }
                crate::open_prompt::advance(app);
                return Ok(json!({"job":job.0}));
            }
            Ok(Started::Done(value)) => {
                finish_probe(app, id, &value)?;
            }
            Err(error) => {
                discard(app, id);
                crate::open_prompt::advance(app);
                return Err(error.to_string());
            }
        }
    }
    crate::open_prompt::advance(app);
    Ok(json!({"heldHistoricalXcfReplacement":id}))
}

fn discard(app: &mut PhotocraftApp, id: u64) {
    app.historical_replacements.held.retain(|held| held.id != id);
    app.prompt_order.retain(|kind| *kind != PromptKind::Replacement(id));
}

fn finish_probe(app: &mut PhotocraftApp, id: u64, value: &Value) -> Result<(), String> {
    let question = match value.get("historical") {
        Some(Value::Null) => None,
        Some(value) => Some(photocraft_io::HistoricalXcfSampleOrderRequest {
            version: value.get("version").and_then(Value::as_u64).and_then(|value| u32::try_from(value).ok()).ok_or("invalid replacement header result")?,
            bits_per_sample: value
                .get("bitsPerSample")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or("invalid replacement header result")?,
        }),
        None => return Err("missing replacement header result".into()),
    };
    let position = app.historical_replacements.held.iter().position(|held| held.id == id).ok_or("replacement source is gone")?;
    let target = app.historical_replacements.held[position].target;
    check(app, target)?;
    if let Some(question) = question {
        let held = &mut app.historical_replacements.held[position];
        held.question = Some(question);
        held.job = None;
    } else {
        // A non-XCF envelope needs no byte-order choice. Keep its reservation until
        // all earlier questions have finished; advance will apply it in order.
        let held = &mut app.historical_replacements.held[position];
        held.job = None;
    }
    Ok(())
}

pub(crate) fn on_event(app: &mut PhotocraftApp, event: &JobEvent) -> bool {
    let Some(id) = app.historical_replacements.held.iter().find(|held| held.job == Some(event.id)).map(|held| held.id) else { return false };
    let result = match &event.outcome {
        JobOutcome::Done(value) => finish_probe(app, id, value),
        JobOutcome::Failed(error) => Err(error.clone()),
        JobOutcome::Cancelled => Err("cancelled".into()),
    };
    if let Err(error) = result {
        discard(app, id);
        app.ui.status_error = !matches!(&event.outcome, JobOutcome::Cancelled);
        app.ui.status = error;
    }
    crate::open_prompt::advance(app);
    true
}

pub(crate) fn cancel_all(app: &mut PhotocraftApp) {
    for held in app.historical_replacements.held.drain(..) {
        if let Some(job) = held.job {
            app.session.cancel_job(job);
        }
    }
}

fn owned_bytes(bytes: Arc<Vec<u8>>) -> Vec<u8> {
    Arc::try_unwrap(bytes).unwrap_or_else(|bytes| bytes.as_ref().clone())
}

fn replace(
    app: &mut PhotocraftApp,
    target: Target,
    name: &str,
    bytes: Vec<u8>,
    order: Option<photocraft_doc::HistoricalXcfSampleOrder>,
) -> Result<Value, String> {
    check(app, target)?;
    let result = app.with_document(target.doc, |app| {
        photocraft_engine::smart_cmds::replace_contents_from_bytes(
            &mut app.session,
            &json!({"layer":target.layer.0,"historicalXcfSampleOrder":order}),
            name,
            bytes,
        )
        .map_err(|error| error.to_string())
    });
    app.sync_views();
    result
}

pub(crate) fn owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(FIELD)
}

pub(crate) fn can_confirm(fields: &serde_json::Map<String, Value>) -> bool {
    !owns(fields) || matches!(fields.get("historicalXcfSampleOrder").and_then(Value::as_str), Some("little-endian" | "big-endian"))
}

pub(crate) fn show(app: &mut PhotocraftApp, id: u64) {
    let Some(position) = app.historical_replacements.held.iter().position(|held| held.id == id) else { return };
    let held = &app.historical_replacements.held[position];
    if held.job.is_some() {
        return;
    }
    let Some(question) = held.question else {
        let held = app.historical_replacements.held.remove(position);
        app.prompt_order.pop_front();
        if let Err(error) = replace(app, held.target, &held.name, owned_bytes(held.bytes), None) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
        crate::open_prompt::advance(app);
        return;
    };
    let fields = [
        (FIELD.into(), json!(id)),
        ("__label".into(), json!("Historical XCF sample order")),
        ("name".into(), json!(held.name)),
        ("version".into(), json!(question.version)),
        ("bitsPerSample".into(), json!(question.bits_per_sample)),
        ("historicalXcfSampleOrder".into(), Value::Null),
    ]
    .into_iter()
    .collect();
    app.ui.open_dialog(crate::state::DialogKind::Command, fields);
}

pub(crate) fn validate(app: &PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
    let id = fields.get(FIELD).and_then(Value::as_u64).ok_or("missing replacement source identity")?;
    if app.prompt_order.front() != Some(&PromptKind::Replacement(id)) {
        return Err("replacement dialog is stale or out of order".into());
    }
    let held = app.historical_replacements.held.iter().find(|held| held.id == id).ok_or("replacement source is gone")?;
    if held.job.is_some() || held.question.is_none() {
        return Err("the replacement header is still being read".into());
    }
    check(app, held.target)?;
    photocraft_engine::file_cmds::historical_xcf_sample_order(&Value::Object(fields.clone()), COMMAND)
        .map_err(|error| error.to_string())?
        .ok_or("choose Little-endian or Big-endian, or Cancel")?;
    Ok(())
}

pub(crate) fn answer(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>, accept: bool) -> Result<Value, String> {
    let id = fields.get(FIELD).and_then(Value::as_u64).ok_or("missing replacement source identity")?;
    if app.prompt_order.front() != Some(&PromptKind::Replacement(id)) {
        return Err("replacement dialog is stale or out of order".into());
    }
    let order = if accept {
        validate(app, fields)?;
        photocraft_engine::file_cmds::historical_xcf_sample_order(&Value::Object(fields.clone()), COMMAND).map_err(|error| error.to_string())?
    } else {
        None
    };
    let position = app.historical_replacements.held.iter().position(|held| held.id == id).ok_or("replacement source is gone")?;
    let held = app.historical_replacements.held.remove(position);
    app.prompt_order.pop_front();
    let result = if accept { replace(app, held.target, &held.name, owned_bytes(held.bytes), order) } else { Ok(json!({"cancelled":true})) };
    crate::open_prompt::advance(app);
    result
}

#[cfg(test)]
mod tests;
