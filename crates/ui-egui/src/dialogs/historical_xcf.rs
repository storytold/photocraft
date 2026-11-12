//! Explicit per-import choices, ordered with existing memory and profile prompts.
use crate::{PhotocraftApp, open_prompt::PromptKind};
use photocraft_engine::jobs::JobId;
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

/// Derive the held-only command from the live reservation, validating the typed choice before any mutation.
fn canonical_params(app: &PhotocraftApp, fields: &serde_json::Map<String, Value>, accept: bool) -> Result<(u64, Value), String> {
    let id = fields.get(FIELD).and_then(Value::as_u64).ok_or("missing historical XCF source identity")?;
    if app.prompt_order.front() != Some(&PromptKind::Historical(id)) || !app.historical_opens.iter().any(|held| held.id == id) {
        return Err("historical XCF dialog is stale or out of order".into());
    }
    let params = if accept {
        let order = photocraft_engine::file_cmds::historical_xcf_sample_order(&Value::Object(fields.clone()), "file.open")
            .map_err(|error| error.to_string())?
            .ok_or("choose Little-endian or Big-endian, or Cancel")?;
        json!({"heldHistoricalXcfOpen":id,"historicalXcfSampleOrder":order.to_string(),"interactive":true})
    } else {
        json!({"heldHistoricalXcfOpen":id,"cancel":true})
    };
    Ok((id, params))
}

/// Judge canonical parameters before releasing a modal or changing its reservation.
/// Dispatch repeats the session gate, so a later denial must also leave the live capability retryable.
fn authorize(app: &PhotocraftApp, params: &Value) -> Result<(), String> {
    if app.automation_input || app.session.authorize.is_some() {
        if let Some(authorize) = &app.services.automation_command {
            authorize("file.open", params)?;
        }
        if let Some(authorize) = app.session.authorize.or(app.services.automation_authorize) {
            authorize("file.open", params).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

pub(crate) fn validate(app: &PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
    let (_, params) = canonical_params(app, fields, true)?;
    authorize(app, &params)
}

pub(crate) fn answer(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>, accept: bool) -> Result<Value, String> {
    let (id, params) = canonical_params(app, fields, accept)?;
    authorize(app, &params)?;
    if !accept {
        let value = app.run("file.open", params)?;
        app.ui.dialogs.retain(|dialog| dialog.fields.get(FIELD).and_then(Value::as_u64) != Some(id));
        app.historical_opens.retain(|held| held.id != id);
        app.prompt_order.pop_front();
        crate::open_prompt::advance(app);
        crate::jobs_ui::end_batch(app);
        return Ok(value);
    }
    // Reserve A's place until its decoded result arrives; B must not pass A's profile prompt.
    if let Some(front) = app.prompt_order.front_mut() {
        *front = PromptKind::Resuming(id);
    }
    let value = match app.run("file.open", params) {
        Ok(value) => value,
        Err(error) => {
            complete(app, id, false);
            return Err(error);
        }
    };
    app.ui.dialogs.retain(|dialog| dialog.fields.get(FIELD).and_then(Value::as_u64) != Some(id));
    if let Some(job) = value.get("job").and_then(Value::as_u64).map(JobId) {
        let held = app.historical_opens.iter_mut().find(|held| held.id == id).ok_or("historical source is gone")?;
        held.job = Some(job);
        let tab = crate::jobs_ui::OpenTab { job, name: held.name.clone(), path: held.path.clone(), slot: held.slot, order: held.order };
        crate::jobs_ui::track_open(app, tab);
        Ok(json!({"job":job.0,"pending":true}))
    } else {
        let held = app.historical_opens.iter().find(|held| held.id == id).ok_or("historical source is gone")?;
        let (name, path, slot, order) = (held.name.clone(), held.path.clone(), held.slot, held.order);
        let result = crate::jobs_ui::finish_open(app, &name, path.as_deref(), &value, slot, order);
        complete(app, id, result.is_ok());
        result.map(|()| value)
    }
}

pub(crate) fn for_job(app: &PhotocraftApp, job: JobId) -> Option<u64> {
    app.historical_opens.iter().find(|held| held.job == Some(job)).map(|held| held.id)
}

/// Replace the exact reservation with the just-arrived decoded open before advancing.
/// The reservation kept `arrive` from admitting later files during inline/background completion.
pub(crate) fn complete(app: &mut PhotocraftApp, id: u64, decoded: bool) {
    let Some(position) = app.prompt_order.iter().position(|kind| *kind == PromptKind::Resuming(id)) else { return };
    if !decoded && app.session.holds_historical_xcf_open(id) {
        // Command/profile authorization refused before consuming the encoded source.
        // Restore only this reservation; retain its modal and all later opens.
        if let Some(kind) = app.prompt_order.get_mut(position) {
            *kind = PromptKind::Historical(id);
        }
        if let Some(held) = app.historical_opens.iter_mut().find(|held| held.id == id) {
            held.job = None;
        }
        crate::open_prompt::advance(app);
        return;
    }
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
