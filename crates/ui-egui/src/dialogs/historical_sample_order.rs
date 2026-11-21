use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Map, Value, json};

use super::{alerts, historical_replace, historical_xcf, params};
use crate::{PhotocraftApp, open_prompt::PromptKind};

const OPEN: &str = "historicalXcfOpen";
const REPLACE: &str = "historicalXcfReplacement";
const ORDER: &str = "historicalXcfSampleOrder";

/// Explicit order for processing workflows that cannot stop for per-file questions.
pub(crate) fn order_choice() -> Param {
    let options = [("", tl!("Unspecified")), ("little-endian", tl!("Little-endian")), ("big-endian", tl!("Big-endian"))]
        .into_iter()
        .map(|(value, label)| (value.to_owned(), Label::from(label)))
        .collect();
    Param::new(ORDER, tl!("Historical XCF Sample Order"), ParamKind::choice(options, Some(String::new())))
}

pub(crate) const FAMILY: params::ParamFamily = params::ParamFamily { check: Some(check), cancel: Some(cancel), ..params::ParamFamily::new(owns, run) };

fn owns(key: &str) -> bool {
    kind(key).is_some()
}

fn kind(key: &str) -> Option<PromptKind> {
    let (prefix, id) = key.split_once('.')?;
    let id = id.parse().ok()?;
    match prefix {
        OPEN => Some(PromptKind::Historical(id)),
        REPLACE => Some(PromptKind::Replacement(id)),
        _ => None,
    }
}

fn identity(kind: PromptKind) -> Option<(String, &'static str, u64)> {
    let (prefix, field, id) = match kind {
        PromptKind::Historical(id) => (OPEN, historical_xcf::FIELD, id),
        PromptKind::Replacement(id) => (REPLACE, historical_replace::FIELD, id),
        _ => return None,
    };
    Some((format!("{prefix}.{id}"), field, id))
}

fn front_dialog(app: &PhotocraftApp) -> Option<DialogId> {
    let (key, ..) = identity(*app.prompt_order.front()?)?;
    Some(dialog_id(key.as_str()))
}

pub(super) fn is_open(app: &PhotocraftApp) -> bool {
    front_dialog(app).is_some_and(|id| app.framework.dialogs.stack().contains(id))
}

pub(super) fn close(app: &mut PhotocraftApp) {
    if let Some(id) = front_dialog(app)
        && app.framework.dialogs.stack().contains(id)
    {
        let ctx = app.ctx().clone();
        app.framework.dialogs.cancel_by_id(&ctx, id);
    }
}

pub(super) fn show(app: &mut PhotocraftApp, kind: PromptKind, name: &str, version: u64, bits: u64) {
    let Some((key, field, id)) = identity(kind) else { return };
    let heading = crate::i18n::fmt(tl!("The older XCF document “{name}” does not declare the byte order of its multibyte samples."), &[("name", name)]);
    let mut fields = DialogFields::new();
    fields.set(field, FieldValue::Text(id.to_string()));
    fields.set("name", FieldValue::Text(name.to_owned()));
    fields.set("version", FieldValue::Int(i64::try_from(version).unwrap_or_default()));
    fields.set("bitsPerSample", FieldValue::Int(i64::try_from(bits).unwrap_or_default()));
    params::open_param_unsaved(app, &key.clone(), fields, Value::Null, move |fields| {
        let choices = vec![("little-endian".to_owned(), Label::from(tl!("Little-endian"))), ("big-endian".to_owned(), Label::from(tl!("Big-endian")))];
        let rows = vec![
            Param::section(heading.clone()),
            Param::new(
                ORDER,
                tl!("Choose the sample byte order used when this file was saved. No order is selected automatically."),
                ParamKind::radio(choices, Some(String::new())),
            ),
        ];
        ParamDialog::command(key.clone(), tl!("Historical XCF sample order"), rows).values(Bind::bound(fields))
    });
}

fn fields(key: &str, form: &DialogFields) -> Option<(PromptKind, Map<String, Value>)> {
    let kind = kind(key)?;
    let (_, field, id) = identity(kind)?;
    let fields = [(field.to_owned(), json!(id)), (ORDER.to_owned(), json!(form.text(ORDER)))].into_iter().collect();
    Some((kind, fields))
}

fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let Some((kind, fields)) = fields(&result.key, &result.fields) else { return Err(Label::from("The historical XCF source is gone.")) };
    match kind {
        PromptKind::Historical(_) => historical_xcf::validate(app, &fields),
        PromptKind::Replacement(_) => historical_replace::validate(app, &fields),
        _ => Ok(()),
    }
    .map_err(Label::from)
}

fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let Some((kind, fields)) = fields(&result.key, &result.fields) else { return };
    let outcome = match kind {
        PromptKind::Historical(_) => historical_xcf::answer(app, &fields, true),
        PromptKind::Replacement(_) => historical_replace::answer(app, &fields, true),
        _ => return,
    };
    alerts::report(app, outcome);
}

fn cancel(app: &mut PhotocraftApp, _ctx: &egui::Context, dialog: DialogId) {
    if front_dialog(app) != Some(dialog) {
        return;
    }
    let outcome = match app.prompt_order.front().copied() {
        Some(PromptKind::Historical(id)) => historical_xcf::answer(app, &[(historical_xcf::FIELD.into(), json!(id))].into_iter().collect(), false),
        Some(PromptKind::Replacement(id)) => historical_replace::answer(app, &[(historical_replace::FIELD.into(), json!(id))].into_iter().collect(), false),
        _ => return,
    };
    alerts::report(app, outcome);
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use craft::ui::dialog::StackDialog;
    use craft::ui::patterns::form_dialogs::HostedParamDialog;

    pub(crate) struct TestDialog {
        pub id: DialogId,
        pub fields: Map<String, Value>,
    }

    pub(crate) fn dialogs(app: &PhotocraftApp) -> Vec<TestDialog> {
        app.framework
            .dialogs
            .open_dialogs()
            .into_iter()
            .map(|dialog| {
                let mut fields = dialog.fields.and_then(|fields| fields.as_object().cloned()).unwrap_or_default();
                if let Some((_, field, id)) = dialog.key.as_deref().and_then(kind).and_then(identity) {
                    fields.insert(field.into(), json!(id));
                    if fields.get(ORDER).and_then(Value::as_str) == Some("") {
                        fields.insert(ORDER.into(), Value::Null);
                    }
                }
                TestDialog { id: dialog.id, fields }
            })
            .collect()
    }

    pub(crate) fn set_order(app: &mut PhotocraftApp, id: DialogId, order: Value) {
        let dialog = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id).unwrap();
        // Exercise the held OK validation with a bad field as well as the public
        // setter's normal path. The public setter refuses invalid choices.
        if let Some(order) = order.as_str()
            && matches!(order, "big-endian" | "little-endian")
        {
            dialog.set_json_field(&egui::Context::default(), ORDER, json!(order)).unwrap();
        } else {
            dialog.set_field(ORDER, FieldValue::Text(order.as_str().unwrap_or_default().to_owned()));
        }
    }

    pub(crate) fn profile_id(app: &PhotocraftApp) -> DialogId {
        app.framework.dialogs.get::<craft::color_ui::ui::ProfileMismatchDialog>().unwrap().id()
    }

    pub(crate) fn confirm(app: &mut PhotocraftApp, id: DialogId) -> Result<(), String> {
        super::super::confirm(app, &egui::Context::default(), id)
    }

    pub(crate) fn cancel(app: &mut PhotocraftApp, id: DialogId) -> Result<(), String> {
        super::super::cancel(app, &egui::Context::default(), id)
    }
}
