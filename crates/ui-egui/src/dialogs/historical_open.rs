//! Ordinary background Open's historical source choice. Encoded bytes stay in the engine.
use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Label;
use photocraft_engine::jobs::JobId;
use serde_json::{Value, json};

use crate::{PhotocraftApp, jobs_ui, open_prompt};

const COMMAND: &str = "file.resolveHistoricalXcfOpen";

#[derive(Clone)]
pub(crate) struct Pending {
    pub id: u64,
    pub name: String,
    pub path: Option<String>,
    pub slot: Option<usize>,
    pub order: Option<jobs_ui::OpenOrder>,
}

fn id(request: u64) -> DialogId {
    dialog_id(("historical-xcf-open", request))
}

fn valid_order(value: &str) -> bool {
    matches!(value, "little-endian" | "big-endian")
}

pub(crate) struct HistoricalOpenDialog {
    request: u64,
    name: String,
    choice: String,
}

impl StackDialog for HistoricalOpenDialog {
    type Result = String;

    fn id(&self) -> DialogId {
        id(self.request)
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<String>> {
        let message =
            crate::i18n::fmt(tl!("The older XCF document “{name}” does not declare the byte order of its multibyte samples."), &[("name", &self.name)]);
        let shown = FormDialog::new(Label::from(tl!("Historical XCF sample order")), Bind::bound(&mut self.choice))
            .ok_reason(Label::from(tl!("Choose Little-endian or Big-endian, or Cancel")))
            .show(ui, |ui, _form, choice| {
                Text::new(message.clone()).show(ui);
                let group = RadioGroup::new(Label::from(tl!("Sample byte order")), Bind::given(choice.clone())).show(ui, |ui, _| {
                    Radio::new("little-endian".to_owned()).label(tl!("Little-endian")).show(ui);
                    Radio::new("big-endian".to_owned()).label(tl!("Big-endian")).show(ui);
                });
                if let Some(value) = group.output.value().cloned() {
                    *choice = value;
                }
                valid_order(choice)
            });
        match (shown.accepted, shown.closed) {
            (Some(choice), _) => Some(DialogResult::Ok(choice)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<String> {
        valid_order(&self.choice).then(|| self.choice.clone())
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"request": self.request, "historicalXcfSampleOrder": self.choice}))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let choice = value.as_str().filter(|value| value.is_empty() || valid_order(value));
        if field != "historicalXcfSampleOrder" || choice.is_none() {
            return Err("choose little-endian or big-endian; the request identity is read-only".into());
        }
        self.choice = choice.unwrap_or_default().to_owned();
        ctx.request_repaint();
        Ok(())
    }
}

pub(crate) fn is_open(app: &PhotocraftApp) -> bool {
    app.held_historical_opens.iter().any(|pending| app.framework.dialogs.stack().contains(id(pending.id)))
}

pub(crate) fn arrive(app: &mut PhotocraftApp, pending: Pending) -> Result<(), String> {
    if !app.session.holds_historical_xcf_open(pending.id) {
        return Err("the historical Open request is no longer available".into());
    }
    if !app.held_historical_opens.iter().any(|held| held.id == pending.id) {
        app.prompt_order.push_back(open_prompt::PromptKind::Historical(pending.id));
        app.held_historical_opens.push_back(pending);
    }
    open_prompt::advance(app);
    Ok(())
}

pub(crate) fn show(app: &mut PhotocraftApp, request: u64) {
    let Some(pending) = app.held_historical_opens.iter().find(|pending| pending.id == request) else { return };
    if app.framework.dialogs.stack().contains(id(request)) {
        return;
    }
    app.framework.dialogs.open(HistoricalOpenDialog { request, name: pending.name.clone(), choice: String::new() });
}

fn remove(app: &mut PhotocraftApp, request: u64) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.cancel_by_id(&ctx, id(request));
    let _ = app.framework.dialogs.take_by_id::<HistoricalOpenDialog>(id(request));
    app.held_historical_opens.retain(|pending| pending.id != request);
    app.prompt_order.retain(|kind| *kind != open_prompt::PromptKind::Historical(request));
}

/// An externally cancelled engine capability must not trap subsequent Opens.
pub(crate) fn discard_stale(app: &mut PhotocraftApp) {
    let stale: Vec<_> =
        app.held_historical_opens.iter().filter(|pending| !app.session.holds_historical_xcf_open(pending.id)).map(|pending| pending.id).collect();
    for request in stale {
        remove(app, request);
    }
}

pub(crate) fn answer(app: &mut PhotocraftApp, request: u64, choice: Option<&str>) -> Result<Value, String> {
    if app.prompt_order.front() != Some(&open_prompt::PromptKind::Historical(request)) {
        return Err("this historical Open is not the current prompt".into());
    }
    let pending = app.held_historical_opens.iter().find(|pending| pending.id == request).cloned().ok_or("the historical Open descriptor is missing")?;
    let params = match choice {
        Some(choice) => json!({"id": request, "historicalXcfSampleOrder": choice, "background": app.background_jobs}),
        None => json!({"id": request, "cancel": true}),
    };
    // Dispatch authorization and startup precede removing the retry capability or
    // descriptor. The engine retains it if inline startup or a gate refuses.
    let result = app.run(COMMAND, params)?;
    remove(app, request);
    let tab = jobs_ui::OpenTab { job: JobId(0), name: pending.name, path: pending.path, slot: pending.slot, order: pending.order };
    if let Some(job) = result.get("job").and_then(Value::as_u64) {
        jobs_ui::track_open(app, jobs_ui::OpenTab { job: JobId(job), ..tab });
    } else if let Some(opened) = result.get("result") {
        jobs_ui::finish_open_tab(app, &tab, opened)?;
    } else if choice.is_none() {
        app.ui.status = crate::i18n::fmt(tl!("Canceled opening {name}"), &[("name", &tab.name)]);
        app.ui.status_error = false;
    }
    open_prompt::advance(app);
    jobs_ui::end_batch(app);
    Ok(result)
}

pub(crate) fn cancel_all(app: &mut PhotocraftApp) -> Result<(), String> {
    let requests: Vec<_> = app.held_historical_opens.iter().map(|pending| pending.id).collect();
    for request in requests {
        app.run(COMMAND, json!({"id": request, "cancel": true}))?;
        remove(app, request);
    }
    Ok(())
}

pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    discard_stale(app);
    let Some(open_prompt::PromptKind::Historical(request)) = app.prompt_order.front().copied() else { return };
    let result = app.framework.dialogs.take_by_id::<HistoricalOpenDialog>(id(request));
    let choice = match result {
        Some(DialogResult::Ok(choice)) => Some(choice),
        Some(_) => None,
        None => return,
    };
    if let Err(error) = answer(app, request, choice.as_deref()) {
        super::alerts::report_error(app, error);
        open_prompt::advance(app);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
