//! Explicit per-import choices, ordered with existing memory and profile prompts.
use crate::{PhotocraftApp, open_prompt::PromptKind};
use photocraft_doc::DocId;
use photocraft_engine::jobs::{JobId, Started};
use serde_json::{Value, json};

pub(super) const FIELD: &str = "__historicalXcfOpen";

pub(crate) struct Historical {
    pub id: u64,
    pub name: String,
    pub path: Option<String>,
    pub slot: Option<usize>,
    pub order: Option<crate::jobs_ui::OpenOrder>,
    pub version: u64,
    pub bits: u64,
    pub job: Option<JobId>,
    pub place: Option<DocId>,
}

pub(crate) fn arrive(app: &mut PhotocraftApp, held: Historical) {
    app.prompt_order.push_back(PromptKind::Historical(held.id));
    app.historical_opens.push(held);
    crate::open_prompt::advance(app);
}

pub(crate) fn arrive_place(app: &mut PhotocraftApp, name: &str, doc: DocId, result: &Value) -> Result<bool, String> {
    let Some(id) = result.get("heldHistoricalXcfPlace").and_then(Value::as_u64) else { return Ok(false) };
    let question = result.get("historicalXcfSampleOrderRequired").ok_or("missing historical XCF requirement")?;
    let version = question.get("version").and_then(Value::as_u64).ok_or("missing historical XCF version")?;
    let bits = question.get("bitsPerSample").and_then(Value::as_u64).ok_or("missing historical XCF precision")?;
    arrive(app, Historical { id, name: name.into(), path: None, slot: None, order: None, version, bits, job: None, place: Some(doc) });
    Ok(true)
}

pub(crate) fn show(app: &mut PhotocraftApp, id: u64) {
    let Some(held) = app.historical_opens.iter().find(|held| held.id == id) else { return };
    super::historical_sample_order::show(app, PromptKind::Historical(id), &held.name.clone(), held.version, held.bits);
}

pub(crate) fn validate(app: &PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
    let id = fields.get(FIELD).and_then(Value::as_u64).ok_or("missing historical XCF source identity")?;
    let held = app.historical_opens.iter().find(|held| held.id == id).ok_or("historical XCF dialog is stale or out of order")?;
    if app.prompt_order.front() != Some(&PromptKind::Historical(id)) {
        return Err("historical XCF dialog is stale or out of order".into());
    }
    if held.place.is_some() {
        app.session.validate_historical_xcf_place(id).map_err(|error| error.to_string())?;
        if app.ui.transform.is_some() || app.transform_preparing.is_some() || app.jobs.place.is_some() {
            return Err("Finish the current placement or transform first".into());
        }
    }
    photocraft_engine::file_cmds::historical_xcf_sample_order(&Value::Object(fields.clone()), "file.open")
        .map_err(|error| error.to_string())?
        .ok_or("choose Little-endian or Big-endian, or Cancel")?;
    Ok(())
}

pub(crate) fn answer(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>, accept: bool) -> Result<Value, String> {
    let id = fields.get(FIELD).and_then(Value::as_u64).ok_or("missing historical XCF source identity")?;
    if app.prompt_order.front() != Some(&PromptKind::Historical(id)) {
        return Err("historical XCF dialog is stale or out of order".into());
    }
    if !accept {
        app.session.cancel_historical_xcf_open(id);
        app.historical_opens.retain(|held| held.id != id);
        app.prompt_order.pop_front();
        crate::open_prompt::advance(app);
        crate::jobs_ui::end_batch(app);
        app.ctx().request_repaint();
        return Ok(json!({"cancelled":true}));
    }
    validate(app, fields)?;
    let order = photocraft_engine::file_cmds::historical_xcf_sample_order(&Value::Object(fields.clone()), "file.open")
        .map_err(|error| error.to_string())?
        .ok_or("choose Little-endian or Big-endian, or Cancel")?;
    if let Some((doc, name)) = app.historical_opens.iter().find(|held| held.id == id).and_then(|held| held.place.map(|doc| (doc, held.name.clone()))) {
        return resume_place(app, id, doc, &name, order);
    }
    // Reserve A's place until its decoded result arrives; B must not pass A's profile prompt.
    if let Some(front) = app.prompt_order.front_mut() {
        *front = PromptKind::Resuming(id);
    }
    let started = app.session.resume_historical_xcf_open(id, order, app.background_jobs).map_err(|error| error.to_string());
    match started {
        Ok(Started::Job(job)) => {
            let held = app.historical_opens.iter_mut().find(|held| held.id == id).ok_or("historical source is gone")?;
            held.job = Some(job);
            let tab = crate::jobs_ui::OpenTab { job, name: held.name.clone(), path: held.path.clone(), slot: held.slot, order: held.order };
            crate::jobs_ui::track_open(app, tab);
            Ok(json!({"job":job.0}))
        }
        Ok(Started::Done(value)) => {
            let held = app.historical_opens.iter().find(|held| held.id == id).ok_or("historical source is gone")?;
            let (name, path, slot, order) = (held.name.clone(), held.path.clone(), held.slot, held.order);
            let result = crate::jobs_ui::finish_open_with_held(app, &name, path.as_deref(), &value, slot, order);
            complete(app, id, result.as_ref().ok().copied().flatten());
            result.map(|_| value)
        }
        Err(error) => {
            if app.session.holds_historical_xcf_open(id) {
                // Confirm may already have closed the modal. Startup retained the
                // encoded source, so restore its reservation and recreate the choice.
                if let Some(front) = app.prompt_order.front_mut() {
                    *front = PromptKind::Historical(id);
                }
                crate::open_prompt::advance(app);
            } else {
                complete(app, id, None);
            }
            Err(error)
        }
    }
}

fn resume_place(app: &mut PhotocraftApp, id: u64, doc: DocId, name: &str, order: photocraft_doc::HistoricalXcfSampleOrder) -> Result<Value, String> {
    let started = app.refocus(doc).and_then(|_| app.session.resume_historical_xcf_open(id, order, app.background_jobs).map_err(|error| error.to_string()));
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            crate::open_prompt::advance(app);
            return Err(error);
        }
    };
    app.historical_opens.retain(|held| held.id != id);
    app.prompt_order.pop_front();
    let ctx = app.ctx().clone();
    let result = match started {
        Started::Job(job) => {
            let result = json!({"job": job.0, "pending": true});
            app.jobs.last_started = Some(job);
            crate::jobs_ui::track_place(app, name, &result);
            Ok(result)
        }
        Started::Done(value) => {
            app.sync_views();
            crate::jobs_ui::finish_place_result(app, &ctx, name, doc, &value).map(|()| value)
        }
    };
    crate::open_prompt::advance(app);
    ctx.request_repaint();
    result
}

pub(crate) fn for_job(app: &PhotocraftApp, job: JobId) -> Option<u64> {
    app.historical_opens.iter().find(|held| held.job == Some(job)).map(|held| held.id)
}

/// Replace the exact reservation with the just-arrived decoded open before advancing.
/// The reservation kept `arrive` from admitting later files during inline/background completion.
pub(crate) fn complete(app: &mut PhotocraftApp, id: u64, decoded: Option<photocraft_doc::DocId>) {
    let Some(position) = app.prompt_order.iter().position(|kind| *kind == PromptKind::Resuming(id)) else { return };
    app.historical_opens.retain(|held| held.id != id);
    let decoded_index = app.prompt_order.iter().take(position).filter(|kind| **kind == PromptKind::Open).count();
    app.prompt_order.remove(position);
    // Only this completion's decoded document may replace its reservation. Success
    // without a new held document leaves unrelated queued opens in their original places.
    if decoded.is_some_and(|document| app.held_opens.back().is_some_and(|held| held.doc.id == document))
        && app.prompt_order.back() == Some(&PromptKind::Open)
        && let Some(held) = app.held_opens.pop_back()
    {
        app.prompt_order.pop_back();
        app.prompt_order.insert(position, PromptKind::Open);
        app.held_opens.insert(decoded_index, held);
    }
    crate::open_prompt::advance(app);
}

pub(crate) fn cancel_all(app: &mut PhotocraftApp) {
    for held in app.historical_opens.drain(..) {
        app.session.cancel_historical_xcf_open(held.id);
        if let Some(job) = held.job {
            app.session.cancel_job(job);
        }
    }
}

#[cfg(test)]
mod tests;
