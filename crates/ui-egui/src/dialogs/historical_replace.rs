//! Sample-order choices for exact picked Smart Object sources.
use crate::{PhotocraftApp, open_prompt::PromptKind};
use photocraft_doc::{DocId, LayerContent, LayerId, SmartContentsId};
use photocraft_engine::jobs::{JobCtx, JobEvent, JobId, JobOutcome, Started, StepWork};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) const FIELD: &str = "__historicalXcfReplacement";
const COMMAND: &str = "layer.smartObjects.replaceContents";

#[derive(Clone, Copy)]
pub(crate) struct Target {
    doc: DocId,
    layer: LayerId,
    contents: SmartContentsId,
    revision: u64,
}

#[derive(Clone, Copy)]
enum Action {
    Replace(Target),
    Relink(Target),
    Place { doc: DocId, layer: Option<LayerId>, revision: u64, linked: bool },
}

impl Action {
    fn command(self) -> &'static str {
        match self {
            Self::Replace(_) => COMMAND,
            Self::Relink(_) => "layer.smartObjects.relinkToFile",
            Self::Place { linked: true, .. } => "file.placeLinked",
            Self::Place { linked: false, .. } => "file.placeEmbedded",
        }
    }

    fn check(self, app: &PhotocraftApp) -> Result<(), String> {
        match self {
            Self::Replace(target) | Self::Relink(target) => check(app, target),
            Self::Place { doc, layer, revision, .. } => {
                let state = app.session.documents().iter().find(|state| state.doc.id == doc).ok_or("the placement document was closed")?;
                if app.ui.transform.is_some() {
                    return Err("Commit or cancel the transform first".into());
                }
                if state.revision != revision || state.active_layer != layer {
                    return Err("the placement target changed; pick the file again".into());
                }
                Ok(())
            }
        }
    }
}

struct Held {
    id: u64,
    action: Action,
    name: String,
    bytes: Arc<Vec<u8>>,
    question: Option<photocraft_io::HistoricalXcfSampleOrderRequest>,
    job: Option<JobId>,
    inherited_order: Option<photocraft_doc::HistoricalXcfSampleOrder>,
}

#[derive(Default)]
pub(crate) struct Pending {
    next: u64,
    held: Vec<Held>,
    applying: Option<JobId>,
}

impl Pending {
    pub(crate) fn len(&self) -> usize {
        self.held.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.held.is_empty() && !self.applying()
    }

    pub(crate) fn applying(&self) -> bool {
        self.applying.is_some()
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
    pick_source(app, Action::Replace(target), name, bytes)
}

pub(crate) fn pick_relink(app: &mut PhotocraftApp, target: Target, path: String, bytes: Vec<u8>) -> Result<Value, String> {
    if cfg!(target_arch = "wasm32") || !std::path::Path::new(&path).is_absolute() {
        return Err(format!("{path}: Relink to File needs a file on disk"));
    }
    pick_source(app, Action::Relink(target), path, bytes)
}

pub(crate) fn pick_place(app: &mut PhotocraftApp, doc: DocId, name: String, bytes: Vec<u8>, linked: bool) -> Result<Value, String> {
    let state = app.session.documents().iter().find(|state| state.doc.id == doc).ok_or("the placement document was closed")?;
    let action = Action::Place { doc, layer: state.active_layer, revision: state.revision, linked };
    pick_source(app, action, name, bytes)
}

fn pick_source(app: &mut PhotocraftApp, action: Action, name: String, bytes: Vec<u8>) -> Result<Value, String> {
    action.check(app)?;
    let inherited_order = if let Action::Relink(target) = action {
        app.with_document(target.doc, |app| {
            photocraft_engine::smart_cmds::relink_sample_order(&app.session, target.layer, &bytes).map_err(|error| error.to_string())
        })?
    } else {
        None
    };
    let wrapped = inherited_order.is_none() && photocraft_engine::jobs::has_compressed_import_signature(&bytes);
    // Even a fixed-output probe can scan an arbitrarily long compressed header.
    // Only plain fixed headers are inspected in the picker callback.
    let question = if wrapped || inherited_order.is_some() {
        None
    } else {
        photocraft_io::historical_xcf_sample_order_request(&name, &bytes).map_err(|error| error.to_string())?
    };
    // Plain modern sources reserve their turn just like probed wrappers. Applying
    // one now would change the revision underneath an earlier unanswered choice.
    if !wrapped && question.is_none() && app.prompt_order.is_empty() && !crate::open_prompt::waiting(app) {
        return apply_source(app, action, &name, bytes, inherited_order);
    }
    let pending = &mut app.historical_replacements;
    let retained = pending.held.iter().try_fold(bytes.capacity(), |total, held| total.checked_add(held.bytes.capacity()));
    if pending.held.len() >= 8 || retained.is_none_or(|bytes| bytes > 512 << 20) {
        return Err("too many picked sources await a sample-order choice; finish or cancel an earlier choice".into());
    }
    let id = pending.next.checked_add(1).ok_or("replacement source identity limit reached")?;
    pending.next = id;
    let bytes = Arc::new(bytes);
    pending.held.push(Held { id, action, name: name.clone(), bytes: bytes.clone(), question, job: None, inherited_order });
    app.prompt_order.push_back(PromptKind::Replacement(id));
    if wrapped {
        let started =
            app.session.start_job_stepped(action.command(), Value::Null, "Reading replacement header", false, ProbeWork { name, bytes }, |_, question| {
                Ok(json!({"historical":question.map(|question| json!({"version":question.version,"bitsPerSample":question.bits_per_sample}))}))
            });
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
    app.historical_replacements.held[position].action.check(app)?;
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
    if app.historical_replacements.applying == Some(event.id) {
        app.historical_replacements.applying = None;
        app.sync_views();
        match &event.outcome {
            JobOutcome::Done(_) => {
                app.ui.status = event.label.clone();
                app.ui.status_error = false;
            }
            JobOutcome::Failed(error) => super::alerts::report_error(app, error.clone()),
            JobOutcome::Cancelled => {
                app.ui.status = crate::i18n::fmt(tl!("Canceled {label}"), &[("label", &event.label)]);
                app.ui.status_error = false;
            }
        }
        crate::open_prompt::advance(app);
        return true;
    }
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
    if let Some(job) = app.historical_replacements.applying.take() {
        app.session.cancel_job(job);
    }
    for held in app.historical_replacements.held.drain(..) {
        if let Some(job) = held.job {
            app.session.cancel_job(job);
        }
    }
}

fn owned_bytes(bytes: Arc<Vec<u8>>) -> Vec<u8> {
    Arc::try_unwrap(bytes).unwrap_or_else(|bytes| bytes.as_ref().clone())
}

fn apply_source(
    app: &mut PhotocraftApp,
    action: Action,
    name: &str,
    bytes: Vec<u8>,
    order: Option<photocraft_doc::HistoricalXcfSampleOrder>,
) -> Result<Value, String> {
    action.check(app)?;
    if let Action::Place { doc, linked, .. } = action {
        app.refocus(doc)?;
        let result = app.place_bytes(name, bytes, linked.then(|| name.to_owned()), order)?;
        if !result.get("pending").and_then(Value::as_bool).unwrap_or(false) {
            crate::notices::io_result_warnings(app, &result, name);
        }
        // Background placement starts Free Transform when the layer lands.
        if result.get("frame").and_then(Value::as_u64).is_none() && !crate::jobs_ui::track_place(app, name, &result) {
            let ctx = app.ctx().clone();
            crate::transform_tool::begin_placed(app, &ctx)?;
        }
        return Ok(result);
    }
    let (Action::Replace(target) | Action::Relink(target)) = action else { return Err("unsupported Smart Object source action".into()) };
    let result = app.with_document(target.doc, |app| {
        if matches!(action, Action::Relink(_)) {
            let expected = photocraft_io::linked::historical_xcf_digest(&bytes);
            let result = app.run(
                "layer.smartObjects.relinkToFile",
                json!({"layer":target.layer.0,"path":name,"historicalXcfSampleOrder":order,"expectedSourceDigest":expected}),
            )?;
            if result["pending"].as_bool() == Some(true) {
                app.historical_replacements.applying = result["job"].as_u64().map(JobId);
            }
            return Ok(result);
        }
        if app.background_jobs && !cfg!(target_arch = "wasm32") {
            let started = photocraft_engine::smart_cmds::start_picked_replacement(&mut app.session, target.layer, name.to_owned(), bytes, order)
                .map_err(|error| error.to_string())?;
            return Ok(match started {
                Started::Job(id) => {
                    app.historical_replacements.applying = Some(id);
                    app.jobs.last_started = Some(id);
                    app.ui.status.clear();
                    app.ui.status_error = false;
                    json!({"job":id.0,"pending":true})
                }
                Started::Done(value) => value,
            });
        }
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

pub(crate) fn show(app: &mut PhotocraftApp, id: u64) {
    let Some(position) = app.historical_replacements.held.iter().position(|held| held.id == id) else { return };
    let held = &app.historical_replacements.held[position];
    if held.job.is_some() {
        return;
    }
    let Some(question) = held.question else {
        let held = app.historical_replacements.held.remove(position);
        app.prompt_order.pop_front();
        if let Err(error) = apply_source(app, held.action, &held.name, owned_bytes(held.bytes), held.inherited_order) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
        crate::open_prompt::advance(app);
        return;
    };
    super::historical_sample_order::show(
        app,
        PromptKind::Replacement(id),
        &held.name.clone(),
        u64::from(question.version),
        u64::from(question.bits_per_sample),
    );
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
    held.action.check(app)?;
    photocraft_engine::file_cmds::historical_xcf_sample_order(&Value::Object(fields.clone()), held.action.command())
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
    let result = if accept { apply_source(app, held.action, &held.name, owned_bytes(held.bytes), order) } else { Ok(json!({"cancelled":true})) };
    crate::open_prompt::advance(app);
    result
}

#[cfg(test)]
mod tests;
