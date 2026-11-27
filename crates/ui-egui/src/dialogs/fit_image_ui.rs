//! Fit Image's dimension rows use hosted number fields and keep their rejected drafts out of the command.

use craft::behavior::value::{CancelReason, EditEvent};
use craft::ui::contract::{Bind, Props as _, Show as _};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamResult};
use craft::ui::runtime::env::Label;
use craft::ui::text::NumberField;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub(crate) const COMMAND: &str = "file.automate.fitImage";
const LIMIT: f64 = 300_000.0;
const INVALID: &str = "Invalid numeric entry. A value between 1 and 300000 is required. Previous value inserted.";
const RANGE: &str = "A value between 1 and 300000 is required. Closest value inserted.";

#[derive(Clone, Copy)]
struct Side {
    key: &'static str,
    accepted: &'static str,
    invalid: &'static str,
    notice: &'static str,
    reset: &'static str,
}
const SIDES: [Side; 2] = [
    Side { key: "width", accepted: "__widthAccepted", invalid: "__widthInvalid", notice: "__widthNotice", reset: "__widthReset" },
    Side { key: "height", accepted: "__heightAccepted", invalid: "__heightInvalid", notice: "__heightNotice", reset: "__heightReset" },
];

pub(crate) fn rows(width: i64, height: i64) -> [Param; 2] {
    [row(SIDES[0], tl!("Width"), width), row(SIDES[1], tl!("Height"), height)]
}

fn row(side: Side, label: &'static str, default: i64) -> Param {
    Param::custom(side.key, label, move |ui, fields, _| {
        let raw = number(fields, side.key);
        let last = fields.int(side.accepted).unwrap_or(default);
        let mut value = Some(raw.map_or(last as f64, |n| n.clamp(1.0, LIMIT)));
        let epoch = fields.int(side.reset).unwrap_or(0);
        let shown = NumberField::new(label, Bind::bound(&mut value)).id_salt((side.key, epoch)).range(1.0..=LIMIT).commit_decimals(0).show(ui);
        let invalid_blur = shown.edit.events.iter().any(|event| matches!(event, EditEvent::Cancel { reason: CancelReason::Blur, .. }));
        let invalid = shown.edit.unparsed || invalid_blur;
        fields.set(side.invalid, invalid);
        if invalid_blur || shown.out_of_range.is_some() {
            fields.set(side.notice, if invalid_blur { "invalid" } else { "range" });
        }
        if let Some(n) = value {
            fields.set(side.accepted, n.round() as i64);
            if !invalid && raw.is_some_and(|n| (1.0..=LIMIT).contains(&n)) || shown.edit.value().is_some() {
                fields.set(side.key, n.round() as i64);
            }
        }
        shown.edit.response
    })
    .opens_with(default)
    .companions(SIDES.iter().flat_map(|side| [side.accepted, side.invalid, side.notice, side.reset]))
    .not_remembered()
}

fn number(fields: &DialogFields, key: &str) -> Option<f64> {
    fields
        .number(key)
        .or_else(|| {
            let text = fields.text(key).filter(|text| text.len() <= 128)?;
            craft::behavior::quantity::Reader::new().read(text, None).ok()
        })
        .filter(|n| n.is_finite())
}

pub(crate) fn command_values(fields: &DialogFields, map: &mut Map<String, Value>) {
    for side in SIDES {
        if let Some(n) = number(fields, side.key) {
            map.insert(side.key.into(), json!(n.clamp(1.0, LIMIT).round() as i64));
        }
    }
}

pub(super) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let invalid = reconcile(app, result.id, &result.fields, true);
    if invalid { Err(Label::from(tl!(INVALID))) } else { Ok(()) }
}

pub(super) fn answer_input(app: &mut PhotocraftApp, id: DialogId) {
    let fields = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).map(|dialog| dialog.fields().clone());
    if let Some(fields) = fields {
        reconcile(app, id, &fields, false);
    }
}

fn reconcile(app: &mut PhotocraftApp, id: DialogId, fields: &DialogFields, submitting: bool) -> bool {
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) else { return false };
    let mut invalid = false;
    let mut clamped = false;
    for side in SIDES {
        let notice = fields.text(side.notice);
        let raw = number(fields, side.key);
        if notice == Some("invalid") || submitting && (fields.bool(side.invalid) == Some(true) || raw.is_none()) {
            invalid = true;
            let last = fields.int(side.accepted).or_else(|| fields.int(side.key)).unwrap_or(1);
            dialog.set_field(side.key, FieldValue::Int(last));
            dialog.set_field(side.reset, FieldValue::Int(fields.int(side.reset).unwrap_or(0).saturating_add(1)));
            dialog.clear_field(side.invalid);
        }
        clamped |= notice == Some("range") || submitting && raw.is_some_and(|n| !(1.0..=LIMIT).contains(&n));
        if notice.is_some() {
            dialog.clear_field(side.notice);
        }
    }
    if invalid || clamped {
        super::alerts::alert(app, if invalid { tl!(INVALID) } else { tl!(RANGE) });
    }
    invalid
}
