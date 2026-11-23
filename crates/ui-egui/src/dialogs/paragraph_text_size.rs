//! Paragraph Text Size: Alt+drag with the Type tool sizes the new paragraph box in a hosted
//! parameter dialog instead of committing the drag (#12044). Width and Height open in document
//! pixels with the size that was dragged, decimals kept (a ~100 px drag prefills 99.5), and the box
//! keeps its top-left where the drag started. OK runs the same `type.create` a plain drag does, so
//! the placeholder text lands selected exactly as it does there; Cancel closes the dialog and
//! creates nothing.
//!
//! Only what the ticket measured is here. No field rule was measured — no unit menu, no minimum,
//! no maximum — so the number goes to `type.create` as typed and the engine refuses a box it
//! cannot make. A drag upwards or to the left of its start keeps today's rule (the box is the
//! rectangle between the two points), and Cancel beyond closing the dialog is not specified.
//!
//! The dialog is a hosted parameter dialog with a key of its own, so `ui.inspect`, `ui.dialog.set`
//! and `ui.dialog.confirm` drive it like the other parameter dialogs. The drag's top-left, the
//! document it was made in and the tool, type options and placeholder preference the new layer is
//! built from are not fields: they are the family's captured value, read by [`run`]. The last of
//! those three is taken when the dialog opens rather than at OK, because the reference application's
//! dialog is modal — the layer keeps what was active at the drag (#12044).

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamResult};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The dialog family's key. The dialog sizes the box for a `type.create`, but it is not that
/// command's own form, so it carries a key of its own and never routes the plain parameter form.
const KEY: &str = "type.paragraphTextSize";
const WIDTH: &str = "width";
const HEIGHT: &str = "height";

/// Whether `key` is the Paragraph Text Size dialog's.
pub fn owns(key: &str) -> bool {
    key == KEY
}

/// The two rows, Width and Height in document pixels, opening at the size that was dragged. One
/// decimal shows a dragged half pixel (99.5) as such; no range is measured, so the field keeps the
/// number typed and `type.create` refuses a box it cannot make.
fn build(w: f64, h: f64) -> impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static {
    move |_fields: &mut DialogFields| {
        let rows = vec![Param::number(WIDTH, tl!("Width"), w, 1.0, 1), Param::number(HEIGHT, tl!("Height"), h, 1.0, 1)];
        ParamDialog::new(KEY, tl!("Paragraph Text Size"), rows)
    }
}

/// Open the dialog on the box an Alt+drag drew: top-left `(x, y)`, size `w` × `h`, in document
/// pixels and exactly as dragged (the fields keep the decimals). The top-left, the document the drag
/// was made in and the settings the new layer is built from are captured for [`run`]: OK creates the
/// text only while that document is still the active one, and with the tool and options that were
/// active at the drag.
///
/// The settings are snapshotted here rather than read at OK because the reference application's
/// dialog is modal: nothing can change the tool (horizontal or vertical) or its options between the
/// Alt+drag and OK, so the box always uses what was active at the drag. A control that changes them
/// while the dialog happens to be open therefore cannot leak into the new layer (#12044).
pub fn open(app: &mut PhotocraftApp, at: [f64; 2], w: f64, h: f64) -> DialogId {
    let captured = json!({
        "at": [at[0], at[1]],
        "document": app.session.active().map(|st| json!(st.doc.id)).unwrap_or(Value::Null),
        "settings": crate::type_tool::TypeSnapshot::live(app),
    });
    let fields = DialogFields::from_pairs([(WIDTH, FieldValue::Number(w)), (HEIGHT, FieldValue::Number(h))]);
    params::open_param_unsaved(app, KEY, fields, captured, build(w, h))
}

/// OK: create the paragraph text the dialog sized, the same layer a plain drag of the same box
/// would have made, and hand the keyboard to its editor as the drag's release does.
pub fn run(app: &mut PhotocraftApp, ctx: &egui::Context, result: ParamResult) {
    let captured = params::captured(app, &result.key);
    // The box was dragged in one document: never put the text into another one that became active since.
    if captured.get("document") != app.session.active().map(|st| json!(st.doc.id)).as_ref() {
        crate::dialogs::alerts::report_error(app, "the document the box was dragged in is no longer the active one");
        return;
    }
    let at = captured.get("at").and_then(Value::as_array);
    let (Some(x), Some(y)) = (at.as_ref().and_then(|a| a.first()).and_then(Value::as_f64), at.as_ref().and_then(|a| a.get(1)).and_then(Value::as_f64)) else {
        crate::dialogs::alerts::report_error(app, "the Paragraph Text Size dialog has no position");
        return;
    };
    let (Some(w), Some(h)) = (result.fields.number(WIDTH), result.fields.number(HEIGHT)) else {
        crate::dialogs::alerts::report_error(app, "the Paragraph Text Size dialog has no size");
        return;
    };
    // The settings are the ones the drag had, not the ones the app has now: the reference dialog is
    // modal, so a control that switched the tool or edited the options while it was open has no say
    // in this layer (#12044).
    let Ok(settings) = serde_json::from_value(captured.get("settings").cloned().unwrap_or(Value::Null)) else {
        crate::dialogs::alerts::report_error(app, "the Paragraph Text Size dialog has no settings snapshot");
        return;
    };
    let mut p = crate::type_tool::create_params(app, true, &settings);
    p["box"] = json!([x, y, w, h]);
    // A refused create already said why in the status bar, as the gesture's own create does.
    if crate::type_tool::create_layer(app, p, None, &settings).is_ok() {
        crate::type_tool::focus_editor(ctx);
    }
}

#[cfg(test)]
mod tests;
