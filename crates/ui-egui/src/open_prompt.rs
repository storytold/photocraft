//! Interactive opening confirmations: damaged PSD data and Color Settings questions.
//! A decoded file is held back while its standard, agent-accessible dialog asks what to do.
//! Damage confirmation comes first; Continue admits the file normally once any profile question
//! is answered, with no persistent save restriction. Cancelling damage discards just that file;
//! cancelling a profile question retains its existing behaviour of cancelling the whole queue.
//! Files that finish reading while a prompt is up wait their turn.

use std::sync::Arc;

use photocraft_doc::{ColorMode, Document};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The field that marks a prompt as this module's.
const FIELD: &str = "__heldOpen";
/// A regular dialog: agents see it in `ui.inspect` and answer through
/// `ui.dialog.confirm` / `ui.dialog.cancel`, just like the profile prompt.
const DAMAGE_FIELD: &str = "__psdDamagedOpen";
const HISTORICAL_FIELD: &str = "__historicalXcfOpen";

#[derive(Clone)]
pub(crate) struct Historical {
    pub id: u64,
    pub name: String,
    pub path: Option<String>,
    pub slot: Option<usize>,
    pub version: u32,
    pub bits: u32,
}

pub(crate) enum Pending {
    Decoded(Box<Held>),
    Historical(Historical),
}
impl Pending {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Decoded(held) => &held.name,
            Self::Historical(held) => &held.name,
        }
    }
    pub(crate) fn decoded(&self) -> Option<&Held> {
        match self {
            Self::Decoded(held) => Some(held),
            _ => None,
        }
    }
    fn decoded_mut(&mut self) -> Option<&mut Held> {
        match self {
            Self::Decoded(held) => Some(held),
            _ => None,
        }
    }
}

/// A file read and decoded, waiting to open.
pub struct Held {
    pub doc: Document,
    pub initial_view: Option<photocraft_io::NativeInitialView>,
    /// The source is read-only (an Affinity file): Save doesn't write back to it.
    pub read_only: bool,
    /// Ask whether to continue before adding a damaged PSD or showing its profile prompt.
    /// Discarded after the answer: it does not become a document or save restriction.
    pub damage_pending: bool,
    pub name: String,
    pub path: Option<String>,
    pub warnings: Vec<String>,
    /// The file's bytes, for Camera Raw to develop from when there is no path.
    pub bytes: Option<Arc<Vec<u8>>>,
    /// The tab position a reopened file keeps.
    pub slot: Option<usize>,
    /// The profile question ([`photocraft_engine::color_cmds::ColorState::open_question`]), or
    /// `None` to open without asking.
    pub question: Option<Value>,
    /// The Color Settings policy was already applied (a background open added it before it had
    /// to wait); otherwise opening applies it.
    pub policy_applied: bool,
}

/// Whether a file that just finished reading has to wait: one earlier in the queue does.
pub fn waiting(app: &PhotocraftApp) -> bool {
    !app.held_opens.is_empty()
}

/// Opens `held` now, or queues it behind the file whose prompt is up.
pub fn arrive(app: &mut PhotocraftApp, held: Held) {
    app.held_opens.push_back(Pending::Decoded(Box::new(held)));
    advance(app);
}

pub(crate) fn arrive_historical(app: &mut PhotocraftApp, held: Historical) {
    app.held_opens.push_back(Pending::Historical(held));
    advance(app);
}

/// Opens queued files in order until one needs its prompt, which is shown if it isn't yet.
fn advance(app: &mut PhotocraftApp) {
    while let Some(front) = app.held_opens.front() {
        if matches!(front, Pending::Historical(_)) {
            if !app.ui.dialogs.iter().any(|dialog| owns_historical(&dialog.fields)) {
                show_historical(app);
            }
            return;
        }
        let Some(front) = front.decoded() else { return };
        if front.damage_pending {
            if !app.ui.dialogs.iter().any(|d| owns_damage(&d.fields)) {
                show_damage(app);
            }
            return;
        }
        if front.question.is_some() {
            if !app.ui.dialogs.iter().any(|d| d.fields.contains_key(FIELD)) {
                show(app);
            }
            return;
        }
        if let Some(Pending::Decoded(held)) = app.held_opens.pop_front() {
            // A failure is shown to the user (`open_failed`); the next file still opens.
            let _ = open(app, *held, None);
        }
    }
}

/// Shows the Embedded Profile Mismatch (or missing profile) prompt for the first queued file.
fn show(app: &mut PhotocraftApp) {
    let Some(front) = app.held_opens.front().and_then(Pending::decoded) else { return };
    let Some(q) = &front.question else { return };
    // The mode reads as the catalogs name it (French "RVB"); the file name goes in last, so
    // nothing in it is taken for a placeholder.
    let mode = tl!(match front.doc.mode {
        ColorMode::Cmyk => "CMYK",
        ColorMode::Grayscale => "Gray",
        _ => "RGB",
    });
    let name = front.name.as_str();
    let working = crate::i18n::fmt(tl!("Working: {working}"), &[("working", q.get("working").and_then(Value::as_str).unwrap_or("?"))]);
    let missing = q.get("missing").and_then(Value::as_bool) == Some(true);
    // A mismatch names the two profiles on the choices they belong to (`prefs_ui::mismatch_body`).
    let message = if missing {
        let what = crate::i18n::fmt(tl!("The document “{name}” does not have an embedded {mode} profile."), &[("mode", mode), ("name", name)]);
        format!("{what}\n{working}")
    } else {
        crate::i18n::fmt(
            tl!("The document “{name}” has an embedded color profile that does not match the current {mode} working space."),
            &[("mode", mode), ("name", name)],
        )
    };
    let action = q.get("action").and_then(Value::as_str).unwrap_or("preserve").to_string();
    let names = json!({
        "embedded": q.get("embedded").and_then(Value::as_str).unwrap_or("?"),
        "working": q.get("working").and_then(Value::as_str).unwrap_or("?"),
        "policyKey": policy_key(front.doc.mode),
    });
    crate::prefs_ui::open_mismatch(app, &message, &action, missing, FIELD, names);
}

/// The Color Settings policy that answers this mode's files when Ask On Mismatch is off.
fn policy_key(mode: ColorMode) -> Option<&'static str> {
    match photocraft_engine::color_cmds::mode_space(mode) {
        photocraft_cms::ColorSpace::Rgb => Some("policyRgb"),
        photocraft_cms::ColorSpace::Cmyk => Some("policyCmyk"),
        photocraft_cms::ColorSpace::Gray => Some("policyGray"),
        _ => None,
    }
}

/// Whether `fields` are one of this module's prompts.
pub fn owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(FIELD)
}

/// The prompt's answer: `Some(action)` (OK) opens the first queued file with it and moves on to
/// the next; `None` (Cancel) drops every queued file and stops the reads still running.
pub fn answer(app: &mut PhotocraftApp, action: Option<&str>) -> Result<Value, String> {
    if app.held_opens.front().is_some_and(|pending| matches!(pending, Pending::Historical(_))) {
        return Err("answer the XCF sample-order choice first".into());
    }
    if app.held_opens.front().and_then(Pending::decoded).is_some_and(|h| h.damage_pending) {
        return Err("answer the damaged PSD confirmation first".into());
    }
    let Some(action) = action else {
        let dropped = app.held_opens.len();
        let name = app.held_opens.front().map(|h| h.name().to_string()).unwrap_or_default();
        for pending in std::mem::take(&mut app.held_opens) {
            if let Pending::Historical(held) = pending {
                let _ = app.session.execute("file.open", json!({"heldHistoricalXcfOpen":held.id,"cancel":true}));
            }
        }
        for job in app.jobs.opens.iter().map(|o| o.job).collect::<Vec<_>>() {
            app.session.cancel_job(job);
        }
        app.ui.status = crate::i18n::fmt(tl!("Cancelled opening {name}"), &[("name", &name)]);
        app.ui.status_error = false;
        return Ok(json!({"cancelled": dropped}));
    };
    let Some(Pending::Decoded(held)) = app.held_opens.pop_front() else { return Err("no decoded file is waiting to open".into()) };
    let r = open(app, *held, Some(action));
    advance(app);
    r
}

/// OK with "Remember this choice": `action` becomes the Color Settings policy for the file's
/// mode and Ask On Mismatch turns off, so files read from now on open without asking. Files
/// already waiting with a mismatch in the same mode were asked before that and get the same
/// answer; any other question still asks.
pub fn answer_remembering(app: &mut PhotocraftApp, action: &str) -> Result<Value, String> {
    if app.held_opens.front().is_some_and(|pending| matches!(pending, Pending::Historical(_))) {
        return Err("answer the XCF sample-order choice first".into());
    }
    if app.held_opens.front().and_then(Pending::decoded).is_some_and(|h| h.damage_pending) {
        return Err("answer the damaged PSD confirmation first".into());
    }
    let Some(key) = app.held_opens.front().and_then(Pending::decoded).and_then(|h| policy_key(h.doc.mode)) else { return answer(app, Some(action)) };
    let policy = match action {
        "convert" => "convert",
        "discard" => "off",
        _ => "preserve",
    };
    app.run("edit.colorSettings", json!({key: policy, "askOnMismatch": false}))?;
    let same = |h: &Held| {
        !h.damage_pending && policy_key(h.doc.mode) == Some(key) && h.question.as_ref().is_some_and(|q| q.get("missing").and_then(Value::as_bool) != Some(true))
    };
    let Some(Pending::Decoded(held)) = app.held_opens.pop_front() else { return Err("no decoded file is waiting to open".into()) };
    let r = open(app, *held, Some(action));
    while app.held_opens.front().and_then(Pending::decoded).is_some_and(same) {
        if let Some(Pending::Decoded(held)) = app.held_opens.pop_front() {
            // A failure is shown to the user (`open_failed`); the next file still opens.
            let _ = open(app, *held, Some(action));
        }
    }
    advance(app);
    r
}

/// Whether the fields belong to the damage confirmation (not the profile prompt).
pub(crate) fn owns_damage(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(DAMAGE_FIELD)
}

/// Shows the standard agent-visible confirmation for the first damaged PSD
/// waiting in the open queue, including its decoding warnings.
fn show_damage(app: &mut PhotocraftApp) {
    let Some(front) = app.held_opens.front().and_then(Pending::decoded).filter(|h| h.damage_pending) else { return };
    let message = format!(
        "{}\n\n{}\n\n{}",
        crate::i18n::fmt(tl!("The document “{name}” may be damaged."), &[("name", &front.name)]),
        tl!("Some layers, pixels or masks could not be read. Do you want to continue opening it?"),
        tl!("If you continue, saving over the original may permanently discard unreadable data. Save a copy to keep the original."),
    );
    let fields = [
        (DAMAGE_FIELD.into(), json!(true)),
        ("__label".into(), json!("Damaged Photoshop document")),
        ("name".into(), json!(front.name)),
        ("message".into(), json!(message)),
        ("warnings".into(), json!(front.warnings)),
    ]
    .into_iter()
    .collect();
    app.ui.open_dialog(crate::state::DialogKind::Command, fields);
}

/// Body of the standard modal; the same fields are visible to automation.
pub(crate) fn damage_body(ui: &mut egui::Ui, fields: &serde_json::Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
    if let Some(warnings) = fields.get("warnings").and_then(Value::as_array) {
        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
            for warning in warnings.iter().filter_map(Value::as_str) {
                ui.label(warning);
            }
        });
    }
}

/// Continue or discard this file only, then move the queue forward. Accepting does not
/// mark the document read-only: Save, Export, scripts and web downloads work normally.
pub(crate) fn answer_damage(app: &mut PhotocraftApp, accept: bool) -> Result<Value, String> {
    let Some(front) = app.held_opens.front_mut().and_then(Pending::decoded_mut).filter(|h| h.damage_pending) else {
        return Err("no damaged PSD is waiting for confirmation".into());
    };
    let name = front.name.clone();
    if accept {
        front.damage_pending = false;
    } else {
        app.held_opens.pop_front();
        app.ui.status = crate::i18n::fmt(tl!("Cancelled opening {name}"), &[("name", &name)]);
        app.ui.status_error = false;
    }
    advance(app);
    Ok(json!({"name": name, "continued": accept}))
}

pub(crate) fn owns_historical(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(HISTORICAL_FIELD)
}
pub(crate) fn historical_can_confirm(fields: &serde_json::Map<String, Value>) -> bool {
    !owns_historical(fields) || matches!(fields.get("historicalXcfSampleOrder").and_then(Value::as_str), Some("little-endian" | "big-endian"))
}
fn show_historical(app: &mut PhotocraftApp) {
    let Some(Pending::Historical(held)) = app.held_opens.front() else { return };
    let fields = [
        (HISTORICAL_FIELD.into(), json!(held.id)),
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
pub(crate) fn historical_body(ui: &mut egui::Ui, fields: &mut serde_json::Map<String, Value>) {
    ui.label(crate::i18n::fmt(
        tl!("The older XCF document “{name}” does not declare the byte order of its multibyte samples."),
        &[("name", fields.get("name").and_then(Value::as_str).unwrap_or_default())],
    ));
    ui.label(tl!("Choose the sample byte order used when this file was saved. No order is selected automatically."));
    let mut order = fields.get("historicalXcfSampleOrder").and_then(Value::as_str).unwrap_or_default().to_string();
    ui.radio_value(&mut order, "little-endian".into(), tl!("Little-endian"));
    ui.radio_value(&mut order, "big-endian".into(), tl!("Big-endian"));
    if !order.is_empty() {
        fields.insert("historicalXcfSampleOrder".into(), json!(order));
    }
}
pub(crate) fn answer_historical(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>, accept: bool) -> Result<Value, String> {
    let Some(Pending::Historical(held)) = app.held_opens.front() else { return Err("no XCF sample-order choice is waiting".into()) };
    if fields.get(HISTORICAL_FIELD).and_then(Value::as_u64) != Some(held.id) {
        return Err("XCF sample-order dialog is stale".into());
    }
    let held = held.clone();
    let result = if accept {
        if !historical_can_confirm(fields) {
            advance(app);
            return Err("choose Little-endian or Big-endian, or Cancel".into());
        }
        // Inline completion can enqueue the decoded profile question immediately.
        // Consume this question first so its reentrant advance cannot recreate it.
        app.held_opens.pop_front();
        app.ui.dialogs.retain(|dialog| dialog.fields.get(HISTORICAL_FIELD).and_then(Value::as_u64) != Some(held.id));
        crate::jobs_ui::resume_historical(app, &held, fields.get("historicalXcfSampleOrder").and_then(Value::as_str).unwrap_or_default())
    } else {
        app.session.execute("file.open", json!({"heldHistoricalXcfOpen":held.id,"cancel":true})).map_err(|error| error.to_string())
    };
    if !accept && result.is_ok() {
        app.held_opens.pop_front();
        app.ui.dialogs.retain(|dialog| dialog.fields.get(HISTORICAL_FIELD).and_then(Value::as_u64) != Some(held.id));
    }
    advance(app);
    result
}

/// Opens a queued file, applying `action` (the prompt's answer) or else the Color Settings
/// policy, then does what any open does afterwards.
fn open(app: &mut PhotocraftApp, held: Held, action: Option<&str>) -> Result<Value, String> {
    if held.damage_pending {
        return Err("damaged PSD cannot be opened before confirmation".into());
    }
    let Held { mut doc, initial_view, read_only, damage_pending: _, name, path, warnings, bytes, slot, question, policy_applied } = held;
    if let Some(view) = &initial_view
        && let Err(error) = view.validate(&doc)
    {
        let error = error.to_string();
        app.open_failed(&name, &error);
        return Err(error);
    }
    let previous_active = app.session.active_index();
    let index = if question.is_some() || policy_applied {
        if let Some(action) = action
            && let Err(e) = app.session.color.resolve_open(&mut doc, action)
        {
            app.open_failed(&name, &e.to_string());
            return Err(e.to_string());
        }
        if let Err(e) = app.session.prepare_imported_document(&mut doc, &photocraft_engine::jobs::JobCtx::new()) {
            app.open_failed(&name, &e.to_string());
            return Err(e.to_string());
        }
        app.session.add_document(doc, None)
    } else {
        match app.session.open_imported_document(doc, None, &photocraft_engine::jobs::JobCtx::new()) {
            Ok((index, _)) => index,
            Err(e) => {
                app.open_failed(&name, &e.to_string());
                return Err(e.to_string());
            }
        }
    };
    if let Some(st) = app.session.active_mut() {
        st.source_read_only = read_only;
    }
    if let Err(error) = app.session.apply_import_initial_view(initial_view) {
        // The initial validation normally makes this infallible: preparation
        // preserves view targets and add_document only remaps the document ID.
        // Still discard only this new admission if those invariants ever change.
        let _ = app.session.close(index);
        if let Some(previous) = previous_active {
            app.session.set_active(previous);
        }
        let error = error.to_string();
        app.open_failed(&name, &error);
        return Err(error);
    }
    // add_document may remap a persisted ID already owned by another open document.
    let document = app.session.documents().get(index).map(|st| st.doc.id).ok_or("the opened document is missing")?;
    let index = match slot {
        Some(slot) => app
            .run("document.move", json!({"document": index, "to": slot}))
            .ok()
            .and_then(|m| m.get("document").and_then(Value::as_u64))
            .map_or(index, |i| i as usize),
        None => index,
    };
    app.session.set_active(index);
    crate::jobs_ui::after_open(app, document, &name, path.as_deref(), &warnings, bytes.as_deref().map(Vec::as_slice))?;
    Ok(json!({"document": index, "action": action}))
}

#[cfg(test)]
mod tests;
