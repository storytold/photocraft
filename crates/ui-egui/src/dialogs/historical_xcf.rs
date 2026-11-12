//! Explicit per-import choices, ordered with existing memory and profile prompts.
use crate::{PhotocraftApp, open_prompt::PromptKind};
use photocraft_engine::jobs::{JobId, Started};
use serde_json::{Value, json};

const FIELD: &str = "__historicalXcfOpen";

pub(crate) struct Historical {
    pub id: u64,
    pub name: String,
    pub path: Option<String>,
    pub slot: Option<usize>,
    pub order: Option<crate::jobs_ui::OpenOrder>,
    pub version: u64,
    pub bits: u64,
    pub job: Option<JobId>,
}

pub(crate) fn owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(FIELD)
}

pub(crate) fn can_confirm(fields: &serde_json::Map<String, Value>) -> bool {
    !owns(fields) || matches!(fields.get("historicalXcfSampleOrder").and_then(Value::as_str), Some("little-endian" | "big-endian"))
}

pub(crate) fn arrive(app: &mut PhotocraftApp, held: Historical) {
    app.prompt_order.push_back(PromptKind::Historical(held.id));
    app.historical_opens.push(held);
    crate::open_prompt::advance(app);
}

pub(crate) fn show(app: &mut PhotocraftApp, id: u64) {
    let Some(held) = app.historical_opens.iter().find(|held| held.id == id) else { return };
    let fields = [
        (FIELD.into(), json!(id)),
        ("__label".into(), json!("Historical XCF sample order")),
        ("name".into(), json!(held.name)),
        ("version".into(), json!(held.version)),
        ("bitsPerSample".into(), json!(held.bits)),
        ("historicalXcfSampleOrder".into(), Value::Null),
    ]
    .into_iter()
    .collect();
    app.ui.open_dialog(crate::state::DialogKind::Command, fields);
}

pub(crate) fn body(ui: &mut egui::Ui, fields: &mut serde_json::Map<String, Value>) {
    let name = fields.get("name").and_then(Value::as_str).unwrap_or_default();
    ui.label(crate::i18n::fmt(tl!("The older XCF document “{name}” does not declare the byte order of its multibyte samples."), &[("name", name)]));
    ui.label(tl!("Choose the sample byte order used when this file was saved. No order is selected automatically."));
    let mut order = fields.get("historicalXcfSampleOrder").and_then(Value::as_str).unwrap_or_default().to_string();
    ui.radio_value(&mut order, "little-endian".into(), tl!("Little-endian"));
    ui.radio_value(&mut order, "big-endian".into(), tl!("Big-endian"));
    if !order.is_empty() {
        fields.insert("historicalXcfSampleOrder".into(), json!(order));
    }
}

pub(crate) fn validate(app: &PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
    let id = fields.get(FIELD).and_then(Value::as_u64).ok_or("missing historical XCF source identity")?;
    if app.prompt_order.front() != Some(&PromptKind::Historical(id)) || !app.historical_opens.iter().any(|held| held.id == id) {
        return Err("historical XCF dialog is stale or out of order".into());
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
        return Ok(json!({"cancelled":true}));
    }
    validate(app, fields)?;
    let order = photocraft_engine::file_cmds::historical_xcf_sample_order(&Value::Object(fields.clone()), "file.open")
        .map_err(|error| error.to_string())?
        .ok_or("choose Little-endian or Big-endian, or Cancel")?;
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
            let result = crate::jobs_ui::finish_open(app, &name, path.as_deref(), &value, slot, order);
            complete(app, id, result.is_ok());
            result.map(|()| value)
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
                complete(app, id, false);
            }
            Err(error)
        }
    }
}

pub(crate) fn for_job(app: &PhotocraftApp, job: JobId) -> Option<u64> {
    app.historical_opens.iter().find(|held| held.job == Some(job)).map(|held| held.id)
}

/// Replace the exact reservation with the just-arrived decoded open before advancing.
/// The reservation kept `arrive` from admitting later files during inline/background completion.
pub(crate) fn complete(app: &mut PhotocraftApp, id: u64, decoded: bool) {
    let Some(position) = app.prompt_order.iter().position(|kind| *kind == PromptKind::Resuming(id)) else { return };
    app.historical_opens.retain(|held| held.id != id);
    let decoded_index = app.prompt_order.iter().take(position).filter(|kind| **kind == PromptKind::Open).count();
    app.prompt_order.remove(position);
    // A successful XCF completion appends exactly one decoded entry. Failure appends none.
    if decoded
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
